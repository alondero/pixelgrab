//! Cross-check the IPC payload shapes serialize to the expected JSON.
//! This is the Rust-side companion to `src/lib/ipc/types.test.ts`.

use pixelgrab_contracts::capture::{CaptureFormat, CaptureResolution};
use pixelgrab_contracts::coordinate::{PhysicalBounds, PhysicalSize};
use pixelgrab_contracts::drag::{
    DragDiagnostics, DragFormat, DragOutcome, DragRequest, DragTargetEffect, DragTargetKind,
};
use pixelgrab_contracts::ipc::{
    CaptureResolutionDto, CommitRequest, CommitResponse, IpcResponse, RequestCaptureIntent,
    RequestCommitIntent, RequestOverlayIntent, SessionSnapshot, StartShelfDragIntent,
    StartShelfDragResult,
};
use pixelgrab_contracts::pin::{OpenPinRequest, PinCommand, PinSource, PinTransform, PinViewModel};
use pixelgrab_contracts::session::SessionState;

#[test]
fn capture_resolution_dto_round_trips() {
    let dto = CaptureResolutionDto {
        format: "virtual_desktop".to_string(),
        bounds: PhysicalBounds::from_xywh(0, 0, 1920, 1080),
        asset_url: "data:image/png;base64,AAA".to_string(),
        capture_id: "id-1".to_string(),
        captured_at_ms: 1_700_000_000_000,
    };
    let json = serde_json::to_string(&dto).expect("serialize");
    assert!(json.contains("\"assetUrl\""));
    assert!(json.contains("\"captureId\""));
    let parsed: CaptureResolutionDto = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.capture_id, "id-1");
}

#[test]
fn ipc_response_ok_serializes() {
    let env: IpcResponse<u32> = IpcResponse::Ok { data: 42 };
    let json = serde_json::to_string(&env).expect("serialize");
    assert!(json.contains("\"status\":\"ok\""));
    assert!(json.contains("\"data\":42"));
}

#[test]
fn ipc_response_err_serializes() {
    let env: IpcResponse<u32> = IpcResponse::Err {
        error: pixelgrab_contracts::PlatformError::new(
            pixelgrab_contracts::PlatformErrorKind::InvalidPayload,
            "bad",
        ),
    };
    let json = serde_json::to_string(&env).expect("serialize");
    assert!(json.contains("\"status\":\"err\""));
    assert!(json.contains("\"kind\":\"invalid_payload\""));
}

#[test]
fn commit_request_carries_commit_flags() {
    let req = CommitRequest {
        crop: PhysicalBounds::from_xywh(10, 20, 300, 400),
        annotations: vec![],
        to_shelf: true,
        to_clipboard: true,
        save_as: false,
    };
    let json = serde_json::to_string(&req).expect("serialize");
    assert!(json.contains("\"toShelf\":true"));
    assert!(json.contains("\"toClipboard\":true"));
    assert!(json.contains("\"saveAs\":false"));
}

#[test]
fn request_intent_serialises_camel_case() {
    let intent = RequestCaptureIntent {
        intent: pixelgrab_contracts::ipc::CaptureIntent::Region,
    };
    let json = serde_json::to_string(&intent).expect("serialize");
    assert!(json.contains("\"intent\":\"region\""));
}

#[test]
fn session_snapshot_serialises_state() {
    let snapshot = SessionSnapshot {
        state: SessionState::Selecting,
        last_capture: None,
        selection: Some(PhysicalBounds::from_xywh(1, 2, 30, 40)),
    };
    let json = serde_json::to_string(&snapshot).expect("serialize");
    assert!(json.contains("\"state\":\"selecting\""));
    assert!(json.contains("\"width\":30"));
}

#[test]
fn overlay_intent_carries_selection() {
    let intent = RequestOverlayIntent {
        selection: PhysicalBounds::from_xywh(0, 0, 100, 200),
    };
    let json = serde_json::to_string(&intent).expect("serialize");
    assert!(json.contains("\"selection\""));
    assert!(json.contains("\"width\":100"));
}

#[test]
fn commit_intent_serialises_with_crop() {
    let intent = RequestCommitIntent {
        crop: PhysicalBounds::from_xywh(0, 0, 800, 600),
        annotations: vec![],
        to_shelf: false,
        to_clipboard: true,
        save_as: false,
    };
    let json = serde_json::to_string(&intent).expect("serialize");
    assert!(json.contains("\"crop\""));
    assert!(json.contains("\"toClipboard\":true"));
}

#[test]
fn commit_intent_carries_annotation_list() {
    use pixelgrab_contracts::annotation::{
        Annotation, AnnotationColor, AnnotationGeometry, AnnotationId, AnnotationStroke,
    };
    let intent = RequestCommitIntent {
        crop: PhysicalBounds::from_xywh(0, 0, 100, 100),
        annotations: vec![
            Annotation::arrow(
                AnnotationId(1),
                pixelgrab_contracts::PhysicalPoint::new(0, 0),
                pixelgrab_contracts::PhysicalPoint::new(50, 50),
                AnnotationColor::Red,
                AnnotationStroke::Medium,
                0,
            ),
            Annotation::rectangle(
                AnnotationId(2),
                pixelgrab_contracts::PhysicalPoint::new(10, 10),
                pixelgrab_contracts::PhysicalSize::new(20, 20),
                AnnotationColor::Blue,
                AnnotationStroke::Thin,
                1,
            ),
            Annotation::numbered_badge(
                AnnotationId(3),
                pixelgrab_contracts::PhysicalPoint::new(80, 80),
                pixelgrab_contracts::BADGE_RADIUS_PX,
                1,
                AnnotationColor::Yellow,
                AnnotationStroke::Thin,
                2,
            ),
        ],
        to_shelf: false,
        to_clipboard: true,
        save_as: false,
    };
    // Round-trip via JSON to confirm every annotation survives the
    // wire shape (camelCase + nested enum tags).
    let json = serde_json::to_string(&intent).expect("serialize");
    let parsed: RequestCommitIntent = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.annotations.len(), 3);
    assert!(matches!(
        parsed.annotations[0].geometry,
        AnnotationGeometry::Arrow { .. }
    ));
    assert!(matches!(
        parsed.annotations[1].geometry,
        AnnotationGeometry::Rectangle { .. }
    ));
    assert!(matches!(
        parsed.annotations[2].geometry,
        AnnotationGeometry::NumberedBadge { .. }
    ));
    assert_eq!(parsed.annotations[2].number, Some(1));
    // camelCase field names must be on the wire (the frontend mirror
    // in `src/lib/ipc/types.ts` asserts the same shape).
    assert!(json.contains("\"zOrder\""));
    assert!(json.contains("\"color\":\"red\""));
}

/// Pin the palette + stroke widths on the Rust side. The frontend
/// mirror in `src/lib/overlay/KonvaStage.svelte::COLOR_HEX` and
/// `src/lib/annotation/AnnotationToolbar.svelte::COLORS/STROKES`
/// MUST agree with these values; a contract test on the TS side
/// covers the frontend half.
#[test]
fn annotation_palette_and_stroke_widths_are_pinned() {
    use pixelgrab_contracts::annotation::{AnnotationColor, AnnotationStroke};
    assert_eq!(AnnotationColor::Red.rgba(), (0xE5, 0x3B, 0x3B, 0xFF));
    assert_eq!(AnnotationColor::Green.rgba(), (0x3B, 0xE5, 0x5C, 0xFF));
    assert_eq!(AnnotationColor::Blue.rgba(), (0x3B, 0x82, 0xE5, 0xFF));
    assert_eq!(AnnotationColor::Yellow.rgba(), (0xF6, 0xE3, 0x3B, 0xFF));
    assert_eq!(AnnotationColor::White.rgba(), (0xFF, 0xFF, 0xFF, 0xFF));
    assert_eq!(AnnotationStroke::Thin.width_px(), 2);
    assert_eq!(AnnotationStroke::Medium.width_px(), 4);
    assert_eq!(AnnotationStroke::Thick.width_px(), 8);
    assert_eq!(pixelgrab_contracts::BADGE_RADIUS_PX, 18);
}

#[test]
fn commit_response_round_trips() {
    let response = CommitResponse {
        outcome: pixelgrab_contracts::ipc::CommitOutcome {
            capture_id: "abc".to_string(),
            shelf_id: None,
            png_path: Some("/tmp/abc.png".to_string()),
            png_bytes: 1024,
            size_bytes: 4096,
            created_at_ms: 1_700_000_000_000,
        },
    };
    let json = serde_json::to_string(&response).expect("serialize");
    assert!(json.contains("\"captureId\":\"abc\""));
    assert!(json.contains("\"pngPath\":\"/tmp/abc.png\""));
    assert!(json.contains("\"pngBytes\":1024"));
    assert!(json.contains("\"sizeBytes\":4096"));
    assert!(json.contains("\"createdAtMs\":1700000000000"));
}

#[test]
fn capture_resolution_known_format() {
    let resolution = CaptureResolution {
        format: CaptureFormat::VirtualDesktop,
        bounds: PhysicalBounds::from_xywh(0, 0, 1920, 1080),
        asset_url: "data:".to_string(),
        capture_id: "id".to_string(),
        captured_at_ms: 1,
    };
    let _ = PhysicalSize::new(1920, 1080);
    let json = serde_json::to_string(&resolution).expect("serialize");
    assert!(json.contains("\"assetUrl\""));
    assert!(json.contains("\"captureId\""));
    assert!(json.contains("\"capturedAtMs\""));
}

#[test]
fn shelf_card_view_round_trips() {
    // The `ShelfCardView` is the payload of the
    // `pixelgrab://shelf-updated` event. The TypeScript mirror in
    // `src/lib/shelf/types.ts` asserts the same shape; this is the
    // Rust-side companion.
    let view = pixelgrab_lib::shelf::ShelfCardView {
        shelf_id: "shelf-id".to_string(),
        capture_id: "capture-id".to_string(),
        png_path: "/cache/capture/capture.png".to_string(),
        size_bytes: 4096,
        created_at_ms: 1_700_000_000_000,
        bounds: PhysicalBounds::from_xywh(0, 0, 320, 240),
        metadata: pixelgrab_contracts::CacheEntryMetadata {
            title: "Example".to_string(),
            note: "first commit".to_string(),
            tags: vec!["tracer-07".to_string()],
        },
    };
    let json = serde_json::to_string(&view).expect("serialize");
    // Field names must be camelCase on the wire.
    assert!(json.contains("\"shelfId\""));
    assert!(json.contains("\"captureId\""));
    assert!(json.contains("\"pngPath\""));
    assert!(json.contains("\"sizeBytes\":4096"));
    assert!(json.contains("\"createdAtMs\":1700000000000"));
    assert!(json.contains("\"bounds\""));
    assert!(json.contains("\"metadata\""));
    // Round-trip.
    let parsed: pixelgrab_lib::shelf::ShelfCardView =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.shelf_id, view.shelf_id);
    assert_eq!(parsed.metadata.title, "Example");
    assert_eq!(parsed.bounds.size.width, 320);
}

#[test]
fn start_shelf_drag_intent_serialises_camel_case() {
    let req = DragRequest {
        capture_id: "capture-1".into(),
        shelf_id: Some("shelf-1".to_string()),
        png_path: "C:/cache/capture.png".into(),
        bgra_pixels: vec![0u8; 4 * 4 * 4],
        width: 4,
        height: 4,
    };
    let intent = StartShelfDragIntent {
        request: req,
        dismiss_on_accepted: true,
    };
    let json = serde_json::to_string(&intent).expect("serialize");
    assert!(json.contains("\"dismissOnAccepted\":true"));
    assert!(json.contains("\"pngPath\":"));
    assert!(json.contains("\"bgraPixels\":"));
    assert!(json.contains("\"captureId\":\"capture-1\""));
}

#[test]
fn start_shelf_drag_result_serialises_terminal_outcome() {
    let diag = DragDiagnostics::started("cap", Some("shelf".to_string()), 1_000)
        .completed(1_500)
        .with_target_effect(DragTargetEffect::Copy)
        .with_target_kind(DragTargetKind::Chromium);
    let result = StartShelfDragResult {
        outcome: DragOutcome::Accepted,
        diagnostics: diag,
        should_dismiss: true,
    };
    let json = serde_json::to_string(&result).expect("serialize");
    assert!(json.contains("\"outcome\":\"accepted\""));
    assert!(json.contains("\"shouldDismiss\":true"));
    assert!(json.contains("\"targetEffect\":\"copy\""));
    assert!(json.contains("\"targetKind\":\"chromium\""));
    assert!(json.contains("\"durationMs\":500"));
}

#[test]
fn drag_request_validates_buffer_length() {
    let req = DragRequest {
        capture_id: "cap".into(),
        shelf_id: None,
        png_path: "C:/cache/cap.png".into(),
        bgra_pixels: vec![0u8; 8],
        width: 4,
        height: 4,
    };
    let result = req.validate();
    assert!(result.is_err());
}

#[test]
fn drag_outcome_dismiss_card_only_for_accepted() {
    assert!(DragOutcome::Accepted.dismiss_card());
    assert!(!DragOutcome::Rejected.dismiss_card());
    assert!(!DragOutcome::Cancelled.dismiss_card());
    assert!(!DragOutcome::Failed.dismiss_card());
}

#[test]
fn drag_format_labels_are_stable() {
    assert_eq!(DragFormat::Hdrop.as_label(), "hdrop");
    assert_eq!(DragFormat::RegisteredPng.as_label(), "registered_png");
    assert_eq!(DragFormat::DibV5.as_label(), "dib_v5");
    assert_eq!(DragFormat::UnicodeText.as_label(), "unicode_text");
}

#[test]
fn pin_view_model_carries_camel_case_fields() {
    let view = PinViewModel {
        id: pixelgrab_contracts::PinId::new("p-1"),
        transform: PinTransform {
            position: pixelgrab_contracts::PhysicalPoint::new(0, 0),
            window_size: PhysicalSize::new(200, 100),
            source_size: PhysicalSize::new(200, 100),
            zoom: 1.0,
            opacity: 0.8,
        },
        source: PinSource {
            capture_id: "c-1".to_string(),
            png_path: Some("/cache/c-1.png".to_string()),
            bounds: PhysicalBounds::from_xywh(0, 0, 200, 100),
        },
    };
    let json = serde_json::to_string(&view).expect("serialize");
    assert!(json.contains("\"windowSize\""));
    assert!(json.contains("\"sourceSize\""));
    assert!(json.contains("\"captureId\""));
    assert!(json.contains("\"pngPath\""));
    let parsed: PinViewModel = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.id.as_str(), "p-1");
    assert_eq!(parsed.transform.zoom, 1.0);
}

#[test]
fn pin_command_carries_tag() {
    let cmd = PinCommand::Drag { dx: 10, dy: 20 };
    let json = serde_json::to_string(&cmd).expect("serialize");
    assert!(json.contains("\"kind\":\"drag\""));
    assert!(json.contains("\"dx\":10"));
    assert!(json.contains("\"dy\":20"));
}

#[test]
fn pin_open_request_serialises_round_trip() {
    let req = OpenPinRequest {
        capture_id: "c-1".to_string(),
        png_path: "/cache/c-1.png".to_string(),
        bounds: PhysicalBounds::from_xywh(0, 0, 200, 100),
        initial_position: Some(pixelgrab_contracts::PhysicalPoint::new(40, 40)),
    };
    let json = serde_json::to_string(&req).expect("serialize");
    assert!(json.contains("\"captureId\""));
    assert!(json.contains("\"pngPath\""));
    assert!(json.contains("\"initialPosition\""));
    let parsed: OpenPinRequest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.capture_id, "c-1");
    assert_eq!(parsed.png_path, "/cache/c-1.png");
    assert_eq!(
        parsed.initial_position,
        Some(pixelgrab_contracts::PhysicalPoint::new(40, 40))
    );
}
