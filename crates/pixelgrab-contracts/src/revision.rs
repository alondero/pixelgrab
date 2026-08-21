//! Reopenable revision metadata: the editable scene persisted alongside
//! each cache entry so the user can reopen a shelf card for
//! non-destructive editing.
//!
//! ## Why a sidecar?
//!
//! `revision.json` lives next to the existing `metadata.json` (user
//! title / note / tags) and `manifest.json` (cache's structural data).
//! Packing annotations + badge counter + tool/style state into the
//! user-facing `metadata.json` would conflate "what the user sees in
//! the editor" with "what the user sees in the shelf card" — and the
//! two pieces have different schemas. The sidecar keeps the wire
//! shape narrow: `metadata.json` carries human-authored labels,
//! `revision.json` carries editor state, `manifest.json` carries the
//! cache's durability fields.
//!
//! ## Schema versioning
//!
//! `REVISION_SCHEMA_VERSION` is pinned at 1. The loader is tolerant of
//! unknown fields (so additive serializer changes don't require a
//! bump) but rejects any version other than the current one. A
//! future version bump returns a typed
//! `revision_unsupported_version` error so the frontend can fall
//! back to flattened-image editing with reduced editability — the
//! acceptance criterion "Unsupported or missing metadata degrades
//! safely to flattened-image editing".
//!
//! See the corresponding persistence helpers in
//! `src-tauri/src/cache/store.rs` (`Cache::read_revision` and
//! `Cache::write_revision`) and the IPC surface in `src-tauri/src/ipc/commands.rs`

use serde::{Deserialize, Serialize};

use crate::annotation::{Annotation, AnnotationColor, AnnotationStroke};
use crate::cache::{CacheEntryMetadata, ShelfId};
use crate::coordinate::{PhysicalBounds, PhysicalSize};
use crate::CaptureId;

/// Current `revision.json` schema version. Bumped when the wire
/// shape changes incompatibly. The loader is tolerant of unknown
/// fields but rejects any version other than this value, surfacing
/// a typed error so the frontend can fall back to flattened-image
/// editing.
pub const REVISION_SCHEMA_VERSION: u32 = 1;

/// The editor scene persisted with every cache entry. Round-trips
/// across `open_revision` IPC so the user can resume a previous
/// edit; silently upgrades to a fresh editor when the file is
/// missing or unparseable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionMetadata {
    /// Schema version. Pinned to `REVISION_SCHEMA_VERSION`; the loader
    /// rejects any other value with a typed error.
    pub schema_version: u32,
    /// Shelf id of the entry that authored this revision. Used by
    /// analytics and to disambiguate when the same capture_id is
    /// revised across multiple sources.
    pub source_shelf_id: ShelfId,
    /// Capture id of the source entry.
    pub source_capture_id: CaptureId,
    /// Final physical crop used to render the entry's PNG. Always
    /// present so the editor can restore the canvas size.
    pub crop: PhysicalBounds,
    /// Pixel size of the frozen crop. Redundant with `crop.size`
    /// but convenient for the frontend's thumbnail logic.
    pub size: PhysicalSize,
    /// Restored annotation list (arrows, rectangles, text, blur, badges).
    /// Empty when the user committed without annotations.
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    /// Next badge number to assign. Restored so the first new badge
    /// after reopen does not collide with an existing one — the
    /// acceptance criterion "Badge numbering continues correctly after
    /// reopening".
    pub badge_counter: u32,
    /// In-flight draft (pointerdown + drag), preserved across the
    /// re-open so the user does not lose a half-drawn shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<Annotation>,
    /// Active draw tool at the moment of the most recent commit.
    pub active_tool: AnnotationTool,
    /// Active color.
    pub active_color: AnnotationColor,
    /// Active stroke width.
    pub active_stroke: AnnotationStroke,
    /// User-authored metadata (title / note / tags). Mirrors the
    /// entry's `CacheEntryMetadata`; persisted here so a reopen
    /// session starts with the same author-visible labels.
    #[serde(default)]
    pub metadata: CacheEntryMetadata,
}

/// Active draw tool. Mirrors the frontend's `AnnotationTool` enum
/// in `src/lib/annotation/types.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationTool {
    /// The select / transform tool (tracer-06).
    Select,
    /// Draw an arrow.
    Arrow,
    /// Draw a rectangle.
    Rectangle,
    /// Place a numbered badge.
    NumberedBadge,
    /// Text label (tracer-05).
    Text,
    /// Blur / redaction (tracer-05).
    Blur,
}

impl RevisionMetadata {
    /// Build a fresh empty revision for a brand-new entry. The source
    /// shelf id and capture id are recorded so a future revision can
    /// point back to the originating entry.
    pub fn empty(
        source_shelf_id: ShelfId,
        source_capture_id: CaptureId,
        crop: PhysicalBounds,
        size: PhysicalSize,
    ) -> Self {
        Self {
            schema_version: REVISION_SCHEMA_VERSION,
            source_shelf_id,
            source_capture_id,
            crop,
            size,
            annotations: Vec::new(),
            badge_counter: 1,
            draft: None,
            active_tool: AnnotationTool::Select,
            active_color: AnnotationColor::Red,
            active_stroke: AnnotationStroke::Medium,
            metadata: CacheEntryMetadata::default(),
        }
    }

    /// Pin the schema version to the current value, clamp the
    /// `badge_counter` to at least 1, and drop any annotation whose
    /// id is zero (a tampered file). The rasterizer clips
    /// out-of-bounds geometry, so we accept annotations that extend
    /// past the canvas edge — the editor's selection / move paths
    /// surface a no-op for unreachable pixels, which is the right
    /// behaviour for a user who has dragged an annotation past the
    /// edge. The loader calls this on every read so a tampered
    /// file can never crash the editor.
    pub fn sanitize(&self) -> Self {
        let annotations = self
            .annotations
            .iter()
            .filter(|a| a.id.0 != 0)
            .cloned()
            .collect();
        let draft = self.draft.as_ref().filter(|a| a.id.0 != 0).cloned();
        Self {
            schema_version: REVISION_SCHEMA_VERSION,
            source_shelf_id: self.source_shelf_id.clone(),
            source_capture_id: self.source_capture_id.clone(),
            crop: self.crop,
            size: self.size,
            annotations,
            badge_counter: self.badge_counter.max(1),
            draft,
            active_tool: self.active_tool,
            active_color: self.active_color,
            active_stroke: self.active_stroke,
            metadata: self.metadata.clone(),
        }
    }
}

/// The wire shape returned by `open_revision`. Paired with the
/// TypeScript mirror in `src/lib/ipc/types.ts` and the contract
/// tests in `src-tauri/tests/ipc_contracts.rs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionContext {
    /// Shelf id of the source entry (the one being reopened).
    pub shelf_id: ShelfId,
    /// Capture id of the source entry.
    pub capture_id: CaptureId,
    /// Absolute path to the source entry's flattened PNG. The
    /// frontend re-displays this as the canvas.
    pub png_path: String,
    /// The restored editor scene.
    pub revision: RevisionMetadata,
    /// Active lock owners on the source entry. The frontend can
    /// sanity-check that `Editor` is the new owner (i.e. this
    /// session owns the lock).
    #[serde(default)]
    pub locks: Vec<crate::cache::LockOwner>,
    /// Stable diagnostic label describing the loader's path. One of
    /// `"full"`, `"flat_fallback"`. The frontend uses this to
    /// surface a "reduced editability" hint when the sidecar was
    /// missing or unparseable.
    pub loader_status: RevisionLoaderStatus,
}

/// Stable diagnostic label describing how the revision metadata was
/// resolved. The frontend reads this to drive a "reduced
/// editability" hint when the fallback path was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionLoaderStatus {
    /// `revision.json` was present and parsed successfully; the
    /// editor has full editability.
    Full,
    /// `revision.json` was missing, unparseable, or carried an
    /// unsupported version; the editor opens with the flattened PNG
    /// only and reduced editability.
    FlatFallback,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinate::PhysicalPoint;

    fn sample(shelf_id: &str, capture_id: &str) -> RevisionMetadata {
        RevisionMetadata::empty(
            shelf_id.to_string(),
            capture_id.to_string(),
            PhysicalBounds::from_xywh(0, 0, 100, 100),
            PhysicalSize::new(100, 100),
        )
    }

    #[test]
    fn empty_revision_has_default_state() {
        let r = sample("shelf-1", "cap-1");
        assert_eq!(r.schema_version, REVISION_SCHEMA_VERSION);
        assert_eq!(r.source_shelf_id, "shelf-1");
        assert_eq!(r.source_capture_id, "cap-1");
        assert!(r.annotations.is_empty());
        assert_eq!(r.badge_counter, 1);
        assert!(r.draft.is_none());
        assert_eq!(r.active_tool, AnnotationTool::Select);
        assert_eq!(r.active_color, AnnotationColor::Red);
        assert_eq!(r.active_stroke, AnnotationStroke::Medium);
    }

    #[test]
    fn sanitize_clamps_badge_counter_to_one() {
        let mut r = sample("shelf-1", "cap-1");
        r.badge_counter = 0;
        let sanitized = r.sanitize();
        assert_eq!(sanitized.badge_counter, 1);
    }

    #[test]
    fn sanitize_pins_schema_version() {
        let mut r = sample("shelf-1", "cap-1");
        r.schema_version = 99;
        let sanitized = r.sanitize();
        assert_eq!(sanitized.schema_version, REVISION_SCHEMA_VERSION);
    }

    #[test]
    fn revision_metadata_round_trips_via_json() {
        let r = sample("shelf-1", "cap-1");
        let json = serde_json::to_string(&r).expect("serialize");
        assert!(json.contains("\"schemaVersion\":1"));
        assert!(json.contains("\"sourceShelfId\":\"shelf-1\""));
        assert!(json.contains("\"sourceCaptureId\":\"cap-1\""));
        let parsed: RevisionMetadata = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed, r);
    }

    #[test]
    fn revision_metadata_tolerates_unknown_fields() {
        let json = r#"{
            "schemaVersion": 1,
            "sourceShelfId": "shelf-1",
            "sourceCaptureId": "cap-1",
            "crop": {"origin": {"x": 0, "y": 0}, "size": {"width": 100, "height": 100}},
            "size": {"width": 100, "height": 100},
            "annotations": [],
            "badgeCounter": 3,
            "activeTool": "arrow",
            "activeColor": "green",
            "activeStroke": "thick",
            "metadata": {"title": "x", "note": "", "tags": []},
            "futureField": "ignored"
        }"#;
        let parsed: RevisionMetadata = serde_json::from_str(json).expect("parse");
        assert_eq!(parsed.badge_counter, 3);
        assert_eq!(parsed.active_tool, AnnotationTool::Arrow);
    }

    #[test]
    fn revision_context_serialises_with_camel_case() {
        let ctx = RevisionContext {
            shelf_id: "shelf-1".to_string(),
            capture_id: "cap-1".to_string(),
            png_path: "/tmp/cap-1/capture.png".to_string(),
            revision: sample("shelf-1", "cap-1"),
            locks: vec![
                crate::cache::LockOwner::Shelf,
                crate::cache::LockOwner::Editor,
            ],
            loader_status: RevisionLoaderStatus::Full,
        };
        let json = serde_json::to_string(&ctx).expect("serialize");
        assert!(json.contains("\"shelfId\":\"shelf-1\""));
        assert!(json.contains("\"captureId\":\"cap-1\""));
        assert!(json.contains("\"pngPath\":"));
        assert!(json.contains("\"loaderStatus\":\"full\""));
        assert!(json.contains("\"locks\":[\"shelf\",\"editor\"]"));
    }

    #[test]
    fn annotation_tool_serialises_snake_case() {
        let tools = [
            (AnnotationTool::Select, "\"select\""),
            (AnnotationTool::Arrow, "\"arrow\""),
            (AnnotationTool::Rectangle, "\"rectangle\""),
            (AnnotationTool::NumberedBadge, "\"numbered_badge\""),
            (AnnotationTool::Text, "\"text\""),
            (AnnotationTool::Blur, "\"blur\""),
        ];
        for (tool, expected) in tools {
            let json = serde_json::to_string(&tool).expect("serialize");
            assert_eq!(
                json, expected,
                "tool {tool:?} should serialise as {expected}"
            );
        }
    }

    #[test]
    fn revision_with_annotations_round_trips() {
        use crate::annotation::{Annotation, AnnotationId};
        let mut r = sample("shelf-1", "cap-1");
        r.annotations = vec![Annotation::arrow(
            AnnotationId(1),
            PhysicalPoint::new(0, 0),
            PhysicalPoint::new(50, 50),
            AnnotationColor::Red,
            AnnotationStroke::Medium,
            0,
        )];
        let json = serde_json::to_string(&r).expect("serialize");
        let parsed: RevisionMetadata = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed.annotations.len(), 1);
        assert_eq!(parsed, r);
    }
}
