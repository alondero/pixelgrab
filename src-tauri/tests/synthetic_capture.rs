//! Integration tests for the synthetic capture pipeline. Drives the platform
//! contract end-to-end and verifies the produced PNG can be written and
//! re-read.

use std::path::PathBuf;
use std::sync::Arc;

use pixelgrab_lib::platform::synthetic::SyntheticPlatform;
use pixelgrab_lib::platform::PixelGrabPlatform;
use pixelgrab_test_support::fs::IsolatedFilesystem;

#[test]
fn synthetic_capture_returns_data_url() {
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::new());
    let request = pixelgrab_contracts::capture::CaptureRequest {
        format: pixelgrab_contracts::capture::CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let resolution = platform.capture(&request).expect("capture");
    assert!(
        resolution.asset_url.starts_with("data:image/png;base64,"),
        "synthetic capture should produce a data URL"
    );
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
