//! Cross-check the IPC payload shapes serialize to the expected JSON.
//! This is the Rust-side companion to `src/lib/ipc/types.test.ts`.

use pixelgrab_contracts::capture::{CaptureFormat, CaptureResolution};
use pixelgrab_contracts::coordinate::{PhysicalBounds, PhysicalSize};
use pixelgrab_contracts::ipc::{
    CaptureResolutionDto, CommitRequest, CommitResponse, IpcResponse, RequestCaptureIntent,
    RequestCommitIntent, RequestOverlayIntent, SessionSnapshot,
};
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
        to_shelf: false,
        to_clipboard: true,
        save_as: false,
    };
    let json = serde_json::to_string(&intent).expect("serialize");
    assert!(json.contains("\"crop\""));
    assert!(json.contains("\"toClipboard\":true"));
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
