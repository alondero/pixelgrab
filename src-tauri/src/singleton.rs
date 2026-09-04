//! Singleton-ownership helpers.
//!
//! Tracer 14 extends the single-instance plugin's argv parsing to
//! route the secondary launch onto the same internal intent the
//! tray menu and global shortcuts use. The frontend listener
//! treats a single `pixelgrab://secondary-launch` event the same
//! regardless of where the user clicked, so all three entry
//! points reach the same handler.
//!
//! The argv grammar mirrors the cross-platform conventions used
//! by other tray-resident tools:
//!
//! - `pixelgrab.exe` (no flags) → `SecondaryLaunchIntent::Default`
//!   (focus the existing window).
//! - `pixelgrab.exe --capture-region` →
//!   `SecondaryLaunchIntent::CaptureRegion`.
//! - `pixelgrab.exe --capture-full-screen` →
//!   `SecondaryLaunchIntent::CaptureFullScreen`.
//! - `pixelgrab.exe --shelf-history` →
//!   `SecondaryLaunchIntent::ShelfHistory`.
//! - `pixelgrab.exe --settings` →
//!   `SecondaryLaunchIntent::OpenSettings`.
//!
//! The parser is tolerant of unknown flags (a verbose
//! `--debug-logging` style flag the user happened to add has no
//! effect on the routing). The intent is fixed by the *first*
//! recognised flag, so `pixelgrab.exe --capture-region --foo`
//! still routes to `CaptureRegion`.

use tauri::{AppHandle, Emitter, Manager, Runtime};

use pixelgrab_contracts::SecondaryLaunchIntent;

/// Stable name for the secondary-launch intent channel. The
/// frontend listener (in `src/App.svelte`) mirrors this constant.
pub const SINGLE_INSTANCE_EVENT: &str = "pixelgrab://secondary-launch";

/// Bring the existing primary instance to the foreground and emit
/// the forwarded intent. Called from the single-instance plugin
/// closure with the parsed [`SecondaryLaunchIntent`].
pub fn forward_to_existing_instance<R: Runtime>(app: &AppHandle<R>, intent: SecondaryLaunchIntent) {
    if let Some(window) = app.get_webview_window("main") {
        // A forwarded capture must freeze the desktop before any PixelGrab
        // UI becomes visible, just like a tray or global-hotkey capture.
        if !matches!(
            &intent,
            SecondaryLaunchIntent::CaptureRegion
                | SecondaryLaunchIntent::CaptureFullScreen
                | SecondaryLaunchIntent::ShelfHistory
        ) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        let _ = window.emit(SINGLE_INSTANCE_EVENT, &intent);
    }
}

/// Parse a secondary launch argv into a typed intent. The
/// matching is case-insensitive on the flag and order-insensitive
/// across the leading argv tokens. The first flag wins so the
/// caller cannot accidentally combine intents.
pub fn parse_launch_intent(argv: &[String]) -> SecondaryLaunchIntent {
    for token in argv.iter().skip(1) {
        if let Some(intent) = classify_flag(token) {
            return intent;
        }
    }
    SecondaryLaunchIntent::Default
}

fn classify_flag(token: &str) -> Option<SecondaryLaunchIntent> {
    // Skip obvious non-flag tokens to keep the parser resilient.
    if !token.starts_with("--") {
        return None;
    }
    match token.trim_start_matches("--").to_ascii_lowercase().as_str() {
        "capture-region" | "capture_region" | "capture" => {
            Some(SecondaryLaunchIntent::CaptureRegion)
        }
        "capture-full-screen" | "capture_full_screen" | "full-screen" | "fullscreen" => {
            Some(SecondaryLaunchIntent::CaptureFullScreen)
        }
        "shelf-history" | "shelf_history" | "shelf" => Some(SecondaryLaunchIntent::ShelfHistory),
        "settings" | "open-settings" | "open_settings" => Some(SecondaryLaunchIntent::OpenSettings),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check: the event name is stable so the frontend
    /// and the Rust core cannot drift apart silently. The frontend
    /// listens for this string in `src/App.svelte` via
    /// `listen("pixelgrab://secondary-launch", ...)`.
    #[test]
    fn single_instance_event_name_is_stable() {
        assert_eq!(SINGLE_INSTANCE_EVENT, "pixelgrab://secondary-launch");
    }

    #[test]
    fn parse_launch_intent_handles_each_flag() {
        let cases = [
            (
                vec!["pixelgrab.exe".to_string()],
                SecondaryLaunchIntent::Default,
            ),
            (
                vec!["pixelgrab.exe".to_string(), "--capture-region".to_string()],
                SecondaryLaunchIntent::CaptureRegion,
            ),
            (
                vec![
                    "pixelgrab.exe".to_string(),
                    "--capture-full-screen".to_string(),
                ],
                SecondaryLaunchIntent::CaptureFullScreen,
            ),
            (
                vec!["pixelgrab.exe".to_string(), "--shelf-history".to_string()],
                SecondaryLaunchIntent::ShelfHistory,
            ),
            (
                vec!["pixelgrab.exe".to_string(), "--settings".to_string()],
                SecondaryLaunchIntent::OpenSettings,
            ),
        ];
        for (argv, expected) in cases {
            assert_eq!(parse_launch_intent(&argv), expected, "argv={argv:?}");
        }
    }

    #[test]
    fn parse_launch_intent_tolerates_unknown_flags() {
        let argv = vec![
            "pixelgrab.exe".to_string(),
            "--debug-logging".to_string(),
            "--shelf-history".to_string(),
        ];
        assert_eq!(
            parse_launch_intent(&argv),
            SecondaryLaunchIntent::ShelfHistory
        );
    }

    #[test]
    fn parse_launch_intent_ignores_positional_args() {
        let argv = vec![
            "pixelgrab.exe".to_string(),
            "config.json".to_string(),
            "--capture-region".to_string(),
        ];
        assert_eq!(
            parse_launch_intent(&argv),
            SecondaryLaunchIntent::CaptureRegion
        );
    }

    #[test]
    fn parse_launch_intent_is_case_insensitive() {
        let argv = vec!["pixelgrab.exe".to_string(), "--CAPTURE-REGION".to_string()];
        assert_eq!(
            parse_launch_intent(&argv),
            SecondaryLaunchIntent::CaptureRegion
        );
    }

    #[test]
    fn parse_launch_intent_first_flag_wins() {
        let argv = vec![
            "pixelgrab.exe".to_string(),
            "--capture-region".to_string(),
            "--settings".to_string(),
        ];
        assert_eq!(
            parse_launch_intent(&argv),
            SecondaryLaunchIntent::CaptureRegion
        );
    }
}
