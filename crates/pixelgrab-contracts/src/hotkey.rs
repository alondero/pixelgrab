//! Hotkey binding contract.
//!
//! Tracer 14 persists the three user-configurable shortcuts (region
//! capture, full-screen capture, shelf toggle) in a single JSON
//! document next to `shelf-preferences.json`. The bindings are
//! independent of any operating-system handle — the Rust core holds
//! the strings, validates them, and hands them to Tauri's global
//! shortcut plugin at runtime.
//!
//! The on-disk document is also the single source for the tray
//! shortcut hints and the in-app status text. Frontend and tray code
//! must never reach for OS-specific translation functions; the
//! formatter helpers in this module produce the canonical human
//! representation.
//!
//! ## Grammar
//!
//! A binding is either an `Option<String>` (None = unbound) or a
//! canonical string. The parser accepts the conventional desktop
//! grammar:
//!
//! ```text
//! Ctrl/Alt/Shift/Win (in any order, possibly comma-separated or
//! joined with `+`) followed by a non-modifier key.
//! ```
//!
//! The accepted aliases are the same as the `tauri-plugin-global-
//! shortcut` upstream:
//!
//! - Ctrl: `ctrl`, `control`, `ctl`, `cmd`, `command`, `commandorcontrol`
//! - Alt: `alt`, `option`, `opt`
//! - Shift: `shift`, `shft`
//! - Win: `win`, `super`, `meta`, `cmd_l` / `cmd_r` (last two not
//!   supported by `global-hotkey`)
//!
//! The main key must be one of:
//!
//! - A single ASCII letter (`A`..`Z`)
//! - A function key (`F1`..`F24`)
//! - A digit `0`..`9`
//! - A navigation / editing key from the supported set
//!   (see [`SUPPORTED_KEYS`])
//!
//! The parser is permissive on case and whitespace but strict on the
//! structural shape — invalid strings round-trip to `None` rather
//! than silently flipping a binding off.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::ipc::HotkeyBindingsDto;
use crate::PlatformError;

/// Single source of truth for hotkey modifier aliases. Loaded from
/// `data/hotkey_modifiers.json` so the frontend (Svelte) and the
/// Rust core parse the same set of synonyms. Edits to the JSON
/// flow to both sides without code changes — `include_str!` keeps
/// the file embedded at compile time, eliminating the runtime
/// miss-distance that produced the drift this issue closed.
const MODIFIER_ALIASES_JSON: &str = include_str!("../data/hotkey_modifiers.json");

/// Parsed shape of [`MODIFIER_ALIASES_JSON`]. Kept private; the
/// runtime table is built by [`modifier_table`] and consumed via
/// [`canonicalise_modifier`] / [`modifier_rank`].
#[derive(Debug, Deserialize)]
struct ModifierAliasTable {
    /// Schema version. Read by tests but not at runtime; the
    /// file is bundled at compile time so any drift is caught at
    /// the `serde_json::from_str` site instead.
    #[serde(default)]
    #[allow(dead_code)]
    schema_version: u32,
    modifiers: Vec<ModifierEntry>,
    /// Canonical-order rank. Read by tests; runtime sort uses
    /// [`ModifierTable::rank`] which is a `Vec<&'static str>`
    /// sharing the leaked canonical names from [`modifier_table`].
    #[allow(dead_code)]
    rank: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModifierEntry {
    canonical: String,
    aliases: Vec<String>,
}

/// Static lookup table mapping a lowercased modifier alias to its
/// canonical name. Built lazily on first use so the JSON parse
/// cost is paid at most once per process.
fn modifier_table() -> &'static ModifierTable {
    static TABLE: OnceLock<ModifierTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let parsed: ModifierAliasTable =
            serde_json::from_str(MODIFIER_ALIASES_JSON).expect("hotkey_modifiers.json must parse");
        let mut map: HashMap<String, &'static str> =
            HashMap::with_capacity(parsed.modifiers.len() * 6);
        let mut canonicals: Vec<&'static str> = Vec::with_capacity(parsed.modifiers.len());
        for entry in parsed.modifiers {
            // Leak the canonical name once per entry so we can
            // hand out `&'static str` without re-leaking on every
            // lookup. Bounded by the modifier count (currently 4).
            let canonical_static: &'static str = Box::leak(entry.canonical.into_boxed_str());
            canonicals.push(canonical_static);
            for alias in entry.aliases {
                map.insert(alias.to_ascii_lowercase(), canonical_static);
            }
            // The canonical name itself maps to itself so
            // `parse_binding("Alt+F4")` round-trips through the
            // same table as `parse_binding("Option+F4")`.
            map.insert(canonical_static.to_ascii_lowercase(), canonical_static);
        }
        ModifierTable {
            aliases: map,
            rank: canonicals,
        }
    })
}

/// Process-lifetime modifier alias table. Constructed once on
/// first use; both the `HashMap<String, &'static str>` for
/// alias→canonical lookups and the rank-ordered `Vec<&'static str>`
/// for canonical-order sorting share the leaked canonical-name
/// strings, so the memory footprint is bounded by the number of
/// canonical modifier names (currently four).
struct ModifierTable {
    aliases: HashMap<String, &'static str>,
    rank: Vec<&'static str>,
}

/// Schema version for the persisted bindings document. Bumped when
/// the wire shape changes incompatibly so a stale file can be
/// rejected and migrated.
pub const HOTKEY_SETTINGS_SCHEMA_VERSION: u32 = 1;

/// Maximum length of a binding string. Tauri and the `global-hotkey`
/// crate never produce bindings this long; the cap is a defence
/// against a runaway UI letting the user paste a 1 KiB blob.
pub const MAX_BINDING_LEN: usize = 64;

/// Actions the user can bind to a shortcut. The order is the
/// canonical storage order (so a JSON round-trip is deterministic)
/// and the wire enum order for the IPC payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    /// Capture a region the user selects on the overlay.
    RegionCapture,
    /// Capture the active monitor at its native resolution.
    FullScreenCapture,
    /// Show or focus the shelf window.
    ShelfToggle,
}

impl HotkeyAction {
    /// All actions in canonical order. Used by the persistence /
    /// round-trip tests so the struct order cannot drift silently.
    pub const ALL: &'static [HotkeyAction] = &[
        HotkeyAction::RegionCapture,
        HotkeyAction::FullScreenCapture,
        HotkeyAction::ShelfToggle,
    ];

    /// Stable wire id used by the tray menu + frontend.
    pub const fn as_id(self) -> &'static str {
        match self {
            HotkeyAction::RegionCapture => "region_capture",
            HotkeyAction::FullScreenCapture => "full_screen_capture",
            HotkeyAction::ShelfToggle => "shelf_toggle",
        }
    }

    /// Default accelerator for this action. Used when the persisted
    /// file is missing, corrupt, or predates the schema.
    pub const fn default_binding(self) -> &'static str {
        match self {
            HotkeyAction::RegionCapture => "CommandOrControl+Shift+S",
            HotkeyAction::FullScreenCapture => "CommandOrControl+Shift+F",
            HotkeyAction::ShelfToggle => "CommandOrControl+Shift+L",
        }
    }

    /// Human-readable label for the action. Surfaced in the tray
    /// menu and the settings UI.
    pub const fn label(self) -> &'static str {
        match self {
            HotkeyAction::RegionCapture => "Capture Region",
            HotkeyAction::FullScreenCapture => "Capture Full Screen",
            HotkeyAction::ShelfToggle => "Toggle Shelf",
        }
    }
}

/// Map a [`HotkeyAction`] onto the matching
/// [`crate::ipc::SecondaryLaunchIntent`] so the global-shortcut
/// plugin's handler closure (and any future tray-driven dispatch)
/// can resolve the intent in one step. Lives next to the source
/// enum so a future addition to [`HotkeyAction`] is forced to
/// extend this mapping before the IPC layer compiles.
impl From<HotkeyAction> for crate::ipc::SecondaryLaunchIntent {
    fn from(action: HotkeyAction) -> Self {
        match action {
            HotkeyAction::RegionCapture => crate::ipc::SecondaryLaunchIntent::CaptureRegion,
            HotkeyAction::FullScreenCapture => crate::ipc::SecondaryLaunchIntent::CaptureFullScreen,
            HotkeyAction::ShelfToggle => crate::ipc::SecondaryLaunchIntent::ShelfHistory,
        }
    }
}

/// Persisted hotkey bindings document. `None` for an action means
/// the shortcut is unbound; the registry must skip registration for
/// unbound actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyBindings {
    /// Schema version. Bumped when the wire shape changes
    /// incompatibly.
    pub schema_version: u32,
    /// Region-capture shortcut. `None` = unbound.
    pub region_capture: Option<String>,
    /// Full-screen capture shortcut. `None` = unbound.
    pub full_screen_capture: Option<String>,
    /// Shelf-toggle shortcut. `None` = unbound.
    pub shelf_toggle: Option<String>,
    /// Whether the user has paused global shortcuts. Persisted so a
    /// user who paused before closing the app finds the app still
    /// paused on next launch.
    #[serde(default)]
    pub paused: bool,
}

impl HotkeyBindings {
    /// Construct the default bindings (one shortcut per action,
    /// unpaused).
    pub fn defaults() -> Self {
        Self {
            schema_version: HOTKEY_SETTINGS_SCHEMA_VERSION,
            region_capture: Some(HotkeyAction::RegionCapture.default_binding().to_string()),
            full_screen_capture: Some(
                HotkeyAction::FullScreenCapture
                    .default_binding()
                    .to_string(),
            ),
            shelf_toggle: Some(HotkeyAction::ShelfToggle.default_binding().to_string()),
            paused: false,
        }
    }

    /// Read the binding for an action.
    pub fn get(&self, action: HotkeyAction) -> Option<&str> {
        match action {
            HotkeyAction::RegionCapture => self.region_capture.as_deref(),
            HotkeyAction::FullScreenCapture => self.full_screen_capture.as_deref(),
            HotkeyAction::ShelfToggle => self.shelf_toggle.as_deref(),
        }
    }

    /// Replace the binding for an action. `None` removes the
    /// shortcut. Returns `true` when the binding was effectively
    /// changed (used by tests to assert no-op rebinds don't churn
    /// the disk).
    pub fn set(&mut self, action: HotkeyAction, binding: Option<String>) -> bool {
        let next = binding.and_then(|raw| parse_binding(&raw));
        let slot = match action {
            HotkeyAction::RegionCapture => &mut self.region_capture,
            HotkeyAction::FullScreenCapture => &mut self.full_screen_capture,
            HotkeyAction::ShelfToggle => &mut self.shelf_toggle,
        };
        if *slot != next {
            *slot = next;
            true
        } else {
            false
        }
    }

    /// Set the `paused` flag. Returns `true` when the value changed.
    pub fn set_paused(&mut self, paused: bool) -> bool {
        if self.paused != paused {
            self.paused = paused;
            true
        } else {
            false
        }
    }

    /// Sanitise the in-memory state. Invalid strings fall back to
    /// `None`; an underspecified schema version is bumped to the
    /// current. Used both at load time and inside transactional
    /// rebinds so a malformed payload cannot crash the registry.
    ///
    /// The returned [`SanitizeReport`] tells callers exactly which
    /// fields were dropped so the IPC layer / tray tooltip can
    /// surface the recovery without leaking user input.
    pub fn sanitize(mut self) -> (Self, SanitizeReport) {
        let mut report = SanitizeReport::default();
        self.region_capture = self
            .region_capture
            .and_then(|raw| match parse_binding(&raw) {
                Some(canonical) => Some(canonical),
                None => {
                    report.dropped_region = Some(raw);
                    None
                }
            });
        self.full_screen_capture =
            self.full_screen_capture
                .and_then(|raw| match parse_binding(&raw) {
                    Some(canonical) => Some(canonical),
                    None => {
                        report.dropped_full_screen = Some(raw);
                        None
                    }
                });
        self.shelf_toggle = self.shelf_toggle.and_then(|raw| match parse_binding(&raw) {
            Some(canonical) => Some(canonical),
            None => {
                report.dropped_shelf_toggle = Some(raw);
                None
            }
        });
        let schema_was_zero = self.schema_version == 0;
        if schema_was_zero {
            self.schema_version = HOTKEY_SETTINGS_SCHEMA_VERSION;
        }
        report.schema_was_zero = schema_was_zero;
        (self, report)
    }
}

impl Default for HotkeyBindings {
    fn default() -> Self {
        Self::defaults()
    }
}

impl From<HotkeyBindings> for HotkeyBindingsDto {
    fn from(b: HotkeyBindings) -> Self {
        Self {
            schema_version: b.schema_version,
            region_capture: b.region_capture,
            full_screen_capture: b.full_screen_capture,
            shelf_toggle: b.shelf_toggle,
            paused: b.paused,
        }
    }
}

impl From<HotkeyBindingsDto> for HotkeyBindings {
    fn from(d: HotkeyBindingsDto) -> Self {
        Self {
            schema_version: d.schema_version,
            region_capture: d.region_capture,
            full_screen_capture: d.full_screen_capture,
            shelf_toggle: d.shelf_toggle,
            paused: d.paused,
        }
    }
}

/// Parse + canonicalise a binding string. Returns `None` when the
/// string is empty / malformed / too long. The canonical form is
/// upper-case modifier names + `+` joiners + a single uppercase
/// main key.
pub fn parse_binding(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_BINDING_LEN {
        return None;
    }

    // `global-hotkey` accepts both `+` and `,` separators. Normalise
    // first so a user typing `Ctrl, Shift, S` parses the same as
    // `Ctrl+Shift+S`.
    let normalised = trimmed.replace(',', "+");
    let parts: Vec<&str> = normalised
        .split('+')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() < 2 && !is_main_key(parts.first().copied().unwrap_or("")) {
        return None;
    }
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = Vec::new();
    let mut main_key: Option<&str> = None;
    for part in &parts {
        if let Some(modifier) = canonicalise_modifier(part) {
            if main_key.is_some() {
                // Two non-modifier keys in the same chord is invalid.
                return None;
            }
            modifiers.push(modifier);
        } else if is_main_key(part) {
            if main_key.is_some() {
                return None;
            }
            main_key = Some(part);
        } else {
            return None;
        }
    }

    let main_key = main_key?;
    if modifiers.is_empty() && !is_function_or_navigation_key(main_key) {
        // A single letter or digit without a modifier is almost
        // always an accidental binding (or a global hot-key that
        // would steal the user's typing). Reject unless the main
        // key is a function / navigation key, which `global-hotkey`
        // explicitly supports as a self-bound accelerator.
        return None;
    }

    // Canonicalise the modifier order so round-trips are
    // deterministic (Ctrl then Alt then Shift then Win).
    modifiers.sort_by_key(|m| modifier_rank(m));
    let mut out = String::with_capacity(trimmed.len());
    for (idx, modifier) in modifiers.iter().enumerate() {
        if idx > 0 {
            out.push('+');
        }
        out.push_str(modifier);
    }
    let main_canonical = canonicalise_main_key(main_key);
    if !modifiers.is_empty() {
        out.push('+');
    }
    out.push_str(&main_canonical);
    Some(out)
}

/// Render a canonical binding back into a tray-menu-friendly
/// display form. For example `COMMANDORCONTROL+SHIFT+S` becomes
/// `Ctrl+Shift+S`, while `F12` stays as `F12`.
pub fn display_binding(canonical: &str) -> String {
    let mut out = String::with_capacity(canonical.len());
    for (idx, part) in canonical.split('+').enumerate() {
        if idx > 0 {
            out.push('+');
        }
        out.push_str(&humanise_token(part));
    }
    out
}

/// Stable id for the persisted hotkey preferences document. The
/// frontend never writes to this path — it only mirrors the IPC
/// payload — but the name is kept stable so tests can find the
/// file.
pub const PRIMARY_FILENAME: &str = "hotkey-bindings.json";
/// Backup slot for the persisted hotkey preferences. See ADR-0007
/// for the rotation policy.
pub const BACKUP_FILENAME: &str = "hotkey-bindings.json.bak";

/// Structured outcome of [`HotkeyBindings::sanitize`]. Lets callers
/// surface a "binding was dropped" message without leaking the
/// original user input through the IPC error string (which would
/// also flush keyboard-input paths through telemetry).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SanitizeReport {
    /// `true` when the persisted schema version was 0 (i.e. the
    /// document predates the schema or was hand-edited). The
    /// registry bumps the stored value to the current schema.
    pub schema_was_zero: bool,
    /// Raw value of `region_capture` when it was dropped.
    pub dropped_region: Option<String>,
    /// Raw value of `full_screen_capture` when it was dropped.
    pub dropped_full_screen: Option<String>,
    /// Raw value of `shelf_toggle` when it was dropped.
    pub dropped_shelf_toggle: Option<String>,
}

impl SanitizeReport {
    /// `true` when every field round-tripped without dropping.
    pub fn is_clean(&self) -> bool {
        self.dropped_region.is_none()
            && self.dropped_full_screen.is_none()
            && self.dropped_shelf_toggle.is_none()
    }
}

/// Build the registry status payload. The frontend mirrors this in
/// the settings panel + accessibility text; tests pin the field
/// set so the wire shape cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyRegistryStatus {
    /// Whether global shortcuts are currently registered. False
    /// when paused OR after a registration failure.
    pub active: bool,
    /// True when shortcuts are paused via the tray / settings.
    pub paused: bool,
    /// The most recent registration error. `None` when the
    /// registry is happy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Conflicting accelerator (when known). Surfaced for the
    /// settings UI to point at the offending field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicting_action: Option<HotkeyAction>,
}

fn canonicalise_modifier(token: &str) -> Option<&'static str> {
    let key = token.to_ascii_lowercase();
    modifier_table().aliases.get(&key).copied()
}

fn modifier_rank(modifier: &str) -> u8 {
    let rank = &modifier_table().rank;
    rank.iter()
        .position(|m| *m == modifier)
        .and_then(|idx| u8::try_from(idx).ok())
        .unwrap_or(255)
}

fn is_main_key(token: &str) -> bool {
    let upper = token.to_ascii_uppercase();
    if upper.is_empty() {
        return false;
    }
    if upper.len() == 1 {
        let ch = upper.chars().next().unwrap();
        return ch.is_ascii_alphabetic() || ch.is_ascii_digit();
    }
    if let Some(num) = upper.strip_prefix('F') {
        if let Ok(n) = num.parse::<u32>() {
            return (1..=24).contains(&n);
        }
        return false;
    }
    SUPPORTED_KEYS.iter().any(|k| *k == upper)
}

fn is_function_or_navigation_key(token: &str) -> bool {
    let upper = token.to_ascii_uppercase();
    if let Some(num) = upper.strip_prefix('F') {
        if let Ok(n) = num.parse::<u32>() {
            return (1..=24).contains(&n);
        }
    }
    SUPPORTED_KEYS.iter().any(|k| *k == upper)
}

fn canonicalise_main_key(token: &str) -> String {
    let upper = token.to_ascii_uppercase();
    let ch = upper.chars().next().unwrap_or('?');
    // Single-character alphabetic / digit / function-key chord
    // shares the same canonical form: the upper-case token.
    if upper.len() == 1 && (ch.is_ascii_alphabetic() || ch.is_ascii_digit()) {
        return upper;
    }
    if let Some(num) = upper.strip_prefix('F') {
        if num.parse::<u32>().is_ok() {
            return upper;
        }
    }
    upper
}

fn humanise_token(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    // Canonical modifier forms render in their short, mixed-case
    // aliases. Keeping this table in sync with `canonicalise_modifier`
    // is the simplest way to guarantee the display form is
    // recognisable across locales. The lookup is case-insensitive so
    // both canonical and uppercased variants short-circuit.
    let upper = token.to_ascii_uppercase();
    let alias = match upper.as_str() {
        "COMMANDORCONTROL" | "CTRL" | "CMD" | "COMMAND" | "CONTROL" | "CTL" => "Ctrl",
        "ALT" | "OPTION" | "OPT" => "Alt",
        "SHIFT" | "SHFT" => "Shift",
        "SUPER" | "WIN" | "META" => "Win",
        _ => "",
    };
    if !alias.is_empty() {
        return alias.to_string();
    }
    let mut chars = token.chars();
    let first = chars.next().unwrap().to_ascii_uppercase();
    let rest: String = chars.collect();
    format!("{}{}", first, rest)
}

/// Validate a proposed binding, returning a structured error when
/// the registry will reject it. Used by the IPC layer so the
/// frontend can render a user-facing explanation without leaking
/// platform internals.
pub fn validate_for_storage(action: HotkeyAction, raw: &str) -> Result<String, PlatformError> {
    if raw.trim().is_empty() {
        // An empty string is a request to unbind — accepted
        // silently.
        return Err(PlatformError::new(
            crate::PlatformErrorKind::InvalidPayload,
            format!("{}: empty binding is not assignable", action.as_id()),
        ));
    }
    parse_binding(raw).ok_or_else(|| {
        PlatformError::new(
            crate::PlatformErrorKind::InvalidPayload,
            format!(
                "{}: {:?} is not a recognised accelerator",
                action.as_id(),
                raw
            ),
        )
    })
}

/// Write a status payload to a buffer using a `Write`. Helper used
/// by the diagnostics-emission shim so the formatter can be reused
/// in tests.
pub fn write_status_line(status: &HotkeyRegistryStatus, sink: &mut String) {
    let _ = writeln!(
        sink,
        "hotkeys active={} paused={} error={}",
        status.active,
        status.paused,
        status.last_error.as_deref().unwrap_or("none")
    );
}

/// Supported navigation / editing keys beyond letters / digits /
/// function keys. Names match the `global-hotkey` crate.
pub const SUPPORTED_KEYS: &[&str] = &[
    "TAB",
    "ENTER",
    "ESCAPE",
    "SPACE",
    "BACKSPACE",
    "DELETE",
    "INSERT",
    "HOME",
    "END",
    "PAGEUP",
    "PAGEDOWN",
    "LEFT",
    "RIGHT",
    "UP",
    "DOWN",
    "NUMLOCK",
    "SCROLLLOCK",
    "CAPSLOCK",
    "PRINTSCREEN",
    "PAUSE",
    "NUMPAD0",
    "NUMPAD1",
    "NUMPAD2",
    "NUMPAD3",
    "NUMPAD4",
    "NUMPAD5",
    "NUMPAD6",
    "NUMPAD7",
    "NUMPAD8",
    "NUMPAD9",
    "NUMPADADD",
    "NUMPADSUB",
    "NUMPADMULT",
    "NUMPADDIV",
    "NUMPADDOT",
    "NUMPADENTER",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn check_round_trip(raw: &str) {
        let parsed = parse_binding(raw).unwrap_or_else(|| panic!("failed to parse {raw:?}"));
        let again = parse_binding(&parsed).expect("canonical must re-parse");
        assert_eq!(parsed, again, "canonical form must round-trip");
        let display = display_binding(&parsed);
        let reparsed = parse_binding(&display).expect("display must re-parse");
        assert_eq!(
            parsed, reparsed,
            "display must canonicalise back to the same form"
        );
    }

    #[test]
    fn defaults_are_well_formed() {
        let b = HotkeyBindings::defaults();
        for action in HotkeyAction::ALL {
            assert!(b.get(*action).is_some(), "{action:?} default missing");
            assert!(parse_binding(b.get(*action).unwrap()).is_some());
        }
        assert_eq!(b.schema_version, HOTKEY_SETTINGS_SCHEMA_VERSION);
        assert!(!b.paused);
    }

    #[test]
    fn parse_accepts_canonical_grammar() {
        for raw in [
            "Ctrl+Shift+S",
            "Ctrl + Shift + S",
            "ctrl, shift, s",
            "CommandOrControl+Shift+F12",
            "Alt+F4",
            "Cmd+Shift+P",
            "Super+Space",
        ] {
            check_round_trip(raw);
        }
    }

    #[test]
    fn parse_canonicalises_modifier_order() {
        let parsed = parse_binding("Shift+Ctrl+S").unwrap();
        assert_eq!(parsed, "CommandOrControl+Shift+S");
        let parsed = parse_binding("Win+Alt+Shift+Ctrl+F12").unwrap();
        assert_eq!(parsed, "CommandOrControl+Alt+Shift+Super+F12");
    }

    #[test]
    fn parse_rejects_bare_letter_without_modifier() {
        for raw in ["S", "F12", "TAB", "Shift+", "", "   "] {
            if raw == "F12" || raw == "TAB" {
                // F12 / nav keys are exempt from the modifier rule.
                assert!(parse_binding(raw).is_some(), "{raw:?}");
            } else {
                assert!(parse_binding(raw).is_none(), "{raw:?}");
            }
        }
    }

    #[test]
    fn parse_rejects_unknown_modifier() {
        assert!(parse_binding("Bogus+S").is_none());
    }

    #[test]
    fn parse_rejects_two_main_keys() {
        assert!(parse_binding("Ctrl+A+B").is_none());
    }

    #[test]
    fn parse_rejects_overlength_input() {
        let huge = "Ctrl+".to_string() + &"A".repeat(MAX_BINDING_LEN);
        assert!(parse_binding(&huge).is_none());
    }

    #[test]
    fn display_replaces_canonical_modifier_with_short_form() {
        assert_eq!(display_binding("COMMANDORCONTROL+SHIFT+S"), "Ctrl+Shift+S");
        assert_eq!(display_binding("ALT+F12"), "Alt+F12");
    }

    #[test]
    fn bindings_round_trip_to_dto() {
        let original = HotkeyBindings::defaults();
        let dto: HotkeyBindingsDto = original.clone().into();
        let recovered: HotkeyBindings = dto.into();
        assert_eq!(recovered, original);
    }

    #[test]
    fn sanitize_drops_malformed_fields() {
        let dirty = HotkeyBindings {
            schema_version: 0,
            region_capture: Some("Bogus+S".to_string()),
            full_screen_capture: Some("Ctrl+Shift+F".to_string()),
            shelf_toggle: None,
            paused: false,
        };
        let (clean, report) = dirty.sanitize();
        assert!(clean.region_capture.is_none());
        assert!(clean.full_screen_capture.is_some());
        assert_eq!(clean.schema_version, HOTKEY_SETTINGS_SCHEMA_VERSION);
        assert!(report.dropped_region.is_some());
        assert!(report.schema_was_zero);
        assert!(!report.is_clean());
    }

    #[test]
    fn sanitize_clean_report_when_no_drops() {
        let (clean, report) = HotkeyBindings::defaults().sanitize();
        for action in HotkeyAction::ALL {
            assert!(clean.get(*action).is_some(), "{action:?}");
        }
        assert!(report.is_clean());
        assert!(!report.schema_was_zero);
    }

    #[test]
    fn set_replaces_only_targeted_action() {
        let mut b = HotkeyBindings::defaults();
        let changed = b.set(HotkeyAction::RegionCapture, Some("Ctrl+Alt+R".to_string()));
        assert!(changed);
        assert_eq!(b.region_capture.as_deref(), Some("CommandOrControl+Alt+R"));
        // Untouched bindings still carry the defaults.
        assert!(b.full_screen_capture.is_some());
    }

    #[test]
    fn set_to_none_unbinds() {
        let mut b = HotkeyBindings::defaults();
        let changed = b.set(HotkeyAction::ShelfToggle, None);
        assert!(changed);
        assert!(b.shelf_toggle.is_none());
    }

    #[test]
    fn set_to_same_value_reports_no_change() {
        let mut b = HotkeyBindings::defaults();
        let changed = b.set(
            HotkeyAction::RegionCapture,
            Some(b.get(HotkeyAction::RegionCapture).unwrap().to_string()),
        );
        assert!(!changed, "no-op rebind must not churn");
    }

    #[test]
    fn set_paused_reports_change() {
        let mut b = HotkeyBindings::defaults();
        // First flip from defaults (false) to true is a change.
        assert!(b.set_paused(true));
        assert!(b.paused);
        // Setting the same value again is a no-op.
        assert!(!b.set_paused(true));
        // Flipping back to false is a change.
        assert!(b.set_paused(false));
        assert!(!b.paused);
    }

    #[test]
    fn validate_for_storage_includes_action_label() {
        let err = validate_for_storage(HotkeyAction::RegionCapture, "bogus").expect_err("invalid");
        let msg = format!("{err:?}");
        assert!(msg.contains("region_capture"));
    }

    /// Tracer 14 follow-up: every alias in
    /// `data/hotkey_modifiers.json` must round-trip through the
    /// Rust parser AND through the TS canonicaliser. The JSON is
    /// shared between both sides (Rust: `include_str!`, TS:
    /// `import ... from`), so iterating the JSON here pins the
    /// "TS + Rust modifier aliases round-trip" acceptance
    /// criterion from issue #46.
    #[test]
    fn modifier_aliases_round_trip_via_shared_json() {
        // Parse the same JSON the TS side imports. A drift
        // between the file and the runtime table would surface
        // as a missing alias here.
        let parsed: super::ModifierAliasTable =
            serde_json::from_str(super::MODIFIER_ALIASES_JSON).expect("shared JSON parses");
        assert!(
            !parsed.modifiers.is_empty(),
            "modifier alias table must not be empty"
        );
        for entry in &parsed.modifiers {
            // Each alias canonicalises to the entry's canonical
            // form via `parse_binding("<alias>+S")`. The
            // canonicalised binding's modifier prefix must equal
            // the canonical name.
            for alias in &entry.aliases {
                let raw = format!("{alias}+S");
                let parsed_binding =
                    parse_binding(&raw).unwrap_or_else(|| panic!("alias {alias:?} must parse"));
                assert!(
                    parsed_binding.starts_with(&entry.canonical),
                    "alias {alias:?} must canonicalise to {}, got {parsed_binding:?}",
                    entry.canonical
                );
            }
            // Canonical name itself round-trips.
            let self_binding =
                parse_binding(&format!("{}+S", entry.canonical)).expect("canonical parses");
            assert!(
                self_binding.starts_with(&entry.canonical),
                "canonical {canonical:?} must self-canonicalise, got {self_binding:?}",
                canonical = entry.canonical
            );
        }
        // Rank order is preserved — canonicalise a chord with
        // every modifier in arbitrary order and check the
        // output is rank-sorted.
        let shuffled = "Super+Shift+Alt+CommandOrControl+S";
        let canonical = parse_binding(shuffled).expect("all-modifier chord parses");
        let parts: Vec<&str> = canonical.split('+').collect();
        let mod_only = &parts[..parts.len() - 1];
        let expected_rank = &parsed.rank;
        assert_eq!(
            mod_only.len(),
            expected_rank.len(),
            "every modifier appears exactly once: got {mod_only:?}"
        );
        for (got, want) in mod_only.iter().zip(expected_rank.iter()) {
            assert_eq!(got, want, "rank order must match the JSON");
        }
    }
}
