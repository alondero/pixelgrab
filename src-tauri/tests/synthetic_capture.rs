//! Integration tests for the synthetic capture pipeline. Drives the platform
//! contract end-to-end and verifies the produced PNG can be written and
//! re-read. Also exercises the synthetic drag source added in tracer-09:
//! the drag contract shares the same synthetic platform path, so the
//! four terminal outcomes and the leak guard live here too.

use std::path::PathBuf;
use std::sync::Arc;

use pixelgrab_contracts::drag::{DragFormat, DragRequest, DragResult};
use pixelgrab_lib::platform::synthetic::SyntheticPlatform;
use pixelgrab_lib::platform::{
    DragOutcomePlan, PixelGrabPlatform, SyntheticDragScript, SyntheticDragSource,
};
use pixelgrab_test_support::fs::IsolatedFilesystem;

#[test]
fn synthetic_capture_without_root_falls_back_to_data_url() {
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::new());
    let request = pixelgrab_contracts::capture::CaptureRequest {
        format: pixelgrab_contracts::capture::CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let resolution = platform.capture(&request).expect("capture");
    assert!(
        resolution.asset_url.starts_with("data:image/png;base64,"),
        "without a cache root the transport falls back to a data URL"
    );
}

#[test]
fn synthetic_capture_with_root_returns_local_asset_path() {
    // Issue #63: with a configured cache root the freeze frame is
    // persisted once and the resolution carries its file path — the
    // multi-megabyte base64 payload never crosses IPC.
    let fs = IsolatedFilesystem::new("synthetic-asset-transport").expect("fs");
    let synthetic = Arc::new(SyntheticPlatform::new());
    synthetic.set_cache_root(fs.root().to_path_buf());
    let platform: Arc<dyn PixelGrabPlatform> = synthetic;

    let request = pixelgrab_contracts::capture::CaptureRequest {
        format: pixelgrab_contracts::capture::CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let resolution = platform.capture(&request).expect("capture");
    assert!(
        !resolution.asset_url.starts_with("data:"),
        "asset url must not be an inline data URL when a root is configured"
    );
    let path = std::path::PathBuf::from(&resolution.asset_url);
    assert!(path.exists(), "frame file exists under frames/");
    assert!(path.starts_with(fs.root()));
    // The frame decodes as a PNG.
    let bytes = std::fs::read(&path).expect("read frame");
    assert!(bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));
}

#[test]
fn synthetic_capture_writes_png_to_isolated_root() {
    let fs = IsolatedFilesystem::new("synthetic-capture").expect("fs");
    let synthetic = Arc::new(SyntheticPlatform::new());
    synthetic.set_cache_root(fs.root().to_path_buf());
    let platform: Arc<dyn PixelGrabPlatform> = synthetic;

    let crop = pixelgrab_contracts::coordinate::PhysicalBounds::from_xywh(0, 0, 64, 64);
    let rgba = vec![0u8; 64 * 64 * 4];
    let path: PathBuf = platform
        .write_png("test-id", crop, &rgba)
        .expect("write_png");
    assert!(path.exists(), "PNG should be written to disk");
    let metadata = std::fs::metadata(&path).expect("metadata");
    assert!(metadata.len() > 0);
}

#[test]
fn synthetic_capture_rejects_invalid_rgba_length() {
    let fs = IsolatedFilesystem::new("synthetic-capture").expect("fs");
    let synthetic = Arc::new(SyntheticPlatform::new());
    synthetic.set_cache_root(fs.root().to_path_buf());
    let platform: Arc<dyn PixelGrabPlatform> = synthetic;

    let crop = pixelgrab_contracts::coordinate::PhysicalBounds::from_xywh(0, 0, 64, 64);
    let result = platform.write_png("bad-id", crop, &[0u8; 8]);
    assert!(result.is_err());
}

#[test]
fn synthetic_capture_requires_cache_root() {
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::new());
    let crop = pixelgrab_contracts::coordinate::PhysicalBounds::from_xywh(0, 0, 16, 16);
    let rgba = vec![0u8; 16 * 16 * 4];
    let result = platform.write_png("no-root", crop, &rgba);
    assert!(result.is_err(), "write_png without cache root should fail");
}

#[test]
fn downcast_returns_synthetic() {
    let synthetic = Arc::new(SyntheticPlatform::new());
    let platform: Arc<dyn PixelGrabPlatform> = synthetic.clone();
    let downcast = SyntheticPlatform::downcast(platform).expect("synthetic");
    assert!(Arc::ptr_eq(&downcast, &synthetic));
    // drop the downcasted Arc explicitly so the type-id check is exercised
    drop(downcast);
}

// =====================================================================
// External drag (tracer-09). The synthetic drag source lives on the
// same synthetic platform, so the drag tests live in this seam.
// =====================================================================

struct DragRepo {
    _fs: IsolatedFilesystem,
    png: PathBuf,
}

impl DragRepo {
    fn new(label: &str) -> Self {
        let fs = IsolatedFilesystem::new(label).expect("isolated fs");
        let png = fs.root().join("capture.png");
        // PNG signature so the file handle is real.
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n").expect("write png");
        Self { _fs: fs, png }
    }

    fn png_path(&self) -> &std::path::Path {
        &self.png
    }
}

fn sample_drag_request(png_path: &std::path::Path) -> DragRequest {
    DragRequest {
        capture_id: "capture-1".into(),
        shelf_id: Some("shelf-1".to_string()),
        png_path: png_path.to_string_lossy().to_string(),
        bgra_pixels: vec![0u8; 8 * 8 * 4],
        width: 8,
        height: 8,
    }
}

#[test]
fn synthetic_drag_rejects_missing_png() {
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::new());
    let req = DragRequest {
        capture_id: "capture-1".into(),
        shelf_id: None,
        png_path: "DOES-NOT-EXIST.png".into(),
        bgra_pixels: vec![0u8; 8 * 8 * 4],
        width: 8,
        height: 8,
    };
    let result = platform.start_drag(&req);
    assert!(result.is_err());
}

#[test]
fn synthetic_drag_stable_script_returns_cancelled() {
    let repo = DragRepo::new("stable-cancelled");
    let platform = Arc::new(SyntheticPlatform::new());
    let req = sample_drag_request(repo.png_path());
    let result: DragResult = platform.start_drag(&req).expect("run");
    assert_eq!(
        result.outcome,
        pixelgrab_contracts::drag::DragOutcome::Cancelled
    );
}

#[test]
fn synthetic_drag_cycle_script_round_trips() {
    let repo = DragRepo::new("cycle");
    let platform = Arc::new(SyntheticPlatform::new());
    let synthetic = SyntheticPlatform::downcast(platform.clone()).expect("synthetic");
    synthetic
        .drag_source()
        .set_script(SyntheticDragScript::Cycle);
    let req = sample_drag_request(repo.png_path());
    let mut seen = Vec::new();
    for _ in 0..4 {
        let r = platform.start_drag(&req).expect("run");
        seen.push(r.outcome);
    }
    assert_eq!(
        seen,
        vec![
            pixelgrab_contracts::drag::DragOutcome::Accepted,
            pixelgrab_contracts::drag::DragOutcome::Rejected,
            pixelgrab_contracts::drag::DragOutcome::Cancelled,
            pixelgrab_contracts::drag::DragOutcome::Failed,
        ]
    );
}

#[test]
fn repeated_drags_do_not_leak_handles() {
    let repo = DragRepo::new("no-leak");
    let platform = Arc::new(SyntheticPlatform::new());
    let synthetic = SyntheticPlatform::downcast(platform.clone()).expect("synthetic");
    synthetic
        .drag_source()
        .set_script(SyntheticDragScript::Cycle);
    let req = sample_drag_request(repo.png_path());
    for _ in 0..16 {
        let _ = platform.start_drag(&req).expect("run");
    }
    let source = synthetic.drag_source();
    assert_eq!(source.call_count(), 16);
    assert!(source.held_paths().is_empty(), "no leaked file handles");
}

#[test]
fn failure_injection_stamps_diagnostics() {
    let repo = DragRepo::new("failure-injection");
    let platform = Arc::new(SyntheticPlatform::new());
    let synthetic = SyntheticPlatform::downcast(platform.clone()).expect("synthetic");
    synthetic
        .drag_source()
        .set_script(SyntheticDragScript::AlwaysFail("io"));
    let req = sample_drag_request(repo.png_path());
    let result = platform.start_drag(&req).expect("run");
    assert_eq!(
        result.outcome,
        pixelgrab_contracts::drag::DragOutcome::Failed
    );
    assert_eq!(result.diagnostics.failure_kind.as_deref(), Some("io"));
}

#[test]
fn format_request_is_recorded_in_diagnostics() {
    let repo = DragRepo::new("format-request");
    let platform = Arc::new(SyntheticPlatform::new());
    let synthetic = SyntheticPlatform::downcast(platform.clone()).expect("synthetic");
    let source: SyntheticDragSource = synthetic.drag_source();
    source.request_format(0, DragFormat::Hdrop, 5);
    source.request_format(0, DragFormat::DibV5, 12);
    let req = sample_drag_request(repo.png_path());
    let result = platform.start_drag(&req).expect("run");
    assert_eq!(result.diagnostics.requested_formats.len(), 2);
    assert_eq!(
        result.diagnostics.requested_formats[0].format,
        DragFormat::Hdrop
    );
    assert_eq!(result.diagnostics.requested_formats[0].at_ms, 5);
    assert_eq!(
        result.diagnostics.requested_formats[1].format,
        DragFormat::DibV5
    );
    assert_eq!(result.diagnostics.requested_formats[1].at_ms, 12);
}

#[test]
fn outcome_plan_translates_to_wire_outcome() {
    use pixelgrab_contracts::drag::DragOutcome;
    assert_eq!(
        DragOutcomePlan::Accepted.to_outcome(),
        DragOutcome::Accepted
    );
    assert_eq!(
        DragOutcomePlan::Rejected.to_outcome(),
        DragOutcome::Rejected
    );
    assert_eq!(
        DragOutcomePlan::Cancelled.to_outcome(),
        DragOutcome::Cancelled
    );
    assert_eq!(DragOutcomePlan::Failed.to_outcome(), DragOutcome::Failed);
}

#[test]
fn cancel_rejected_cancelled_and_failed_retain_card() {
    use pixelgrab_contracts::drag::DragOutcome;
    // The "dismiss card" rule is the only one that drives UX; the
    // other three outcomes must return false so the shelf keeps the
    // card for retry.
    for outcome in [
        DragOutcome::Rejected,
        DragOutcome::Cancelled,
        DragOutcome::Failed,
    ] {
        assert!(
            !outcome.dismiss_card(),
            "outcome {outcome:?} must retain card"
        );
    }
    assert!(DragOutcome::Accepted.dismiss_card());
}
