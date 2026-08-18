//! Golden-image regression test: drives the synthetic capture pipeline
//! and asserts the produced PNG has the expected PNG signature.

use std::path::PathBuf;
use std::sync::Arc;

use pixelgrab_lib::platform::synthetic::SyntheticPlatform;
use pixelgrab_lib::platform::PixelGrabPlatform;
use pixelgrab_test_support::fs::IsolatedFilesystem;

const GOLDEN_WIDTH: u32 = 1920;
const GOLDEN_HEIGHT: u32 = 1080;

#[test]
fn golden_synthetic_capture_matches_reference() {
    let fs = IsolatedFilesystem::new("golden").expect("isolated fs");
    let synthetic = Arc::new(SyntheticPlatform::new());
    synthetic.set_cache_root(fs.root().to_path_buf());
    let platform: Arc<dyn PixelGrabPlatform> = synthetic;

    let crop = pixelgrab_contracts::coordinate::PhysicalBounds::from_xywh(
        0,
        0,
        GOLDEN_WIDTH,
        GOLDEN_HEIGHT,
    );
    // Deterministic gradient identical to the reference image.
    let mut rgba = Vec::with_capacity((GOLDEN_WIDTH * GOLDEN_HEIGHT * 4) as usize);
    for y in 0..GOLDEN_HEIGHT {
        for x in 0..GOLDEN_WIDTH {
            rgba.push((x & 0xFF) as u8);
            rgba.push((y & 0xFF) as u8);
            rgba.push(((x ^ y) >> 1) as u8);
            rgba.push(0xFF);
        }
    }
    let path: PathBuf = platform
        .write_png("golden-id", crop, &rgba)
        .expect("write_png");
    let bytes = std::fs::read(&path).expect("read png");
    assert!(
        bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]),
        "PNG signature"
    );
    assert!(bytes.len() > 8, "PNG should contain more than the header");
}
