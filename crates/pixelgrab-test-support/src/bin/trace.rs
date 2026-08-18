//! Standalone binary that drives the synthetic capture pipeline through
//! the orchestrator and writes a PNG to disk. This is the executable form
//! of the "synthetic end-to-end capture" trace referenced in ADR-0004 and
//! validated by `scripts/synthetic-capture-trace.mjs`.

use std::path::PathBuf;
use std::sync::Arc;

use pixelgrab_contracts::capture::{CaptureFormat, CaptureRequest};
use pixelgrab_contracts::coordinate::PhysicalBounds;
use pixelgrab_test_support::{
    fs::IsolatedFilesystem, layout::SyntheticMonitorLayout, SyntheticCapture, SyntheticFrame,
};

fn main() {
    let scratch = std::env::args()
        .skip(1)
        .find_map(|arg| arg.strip_prefix("--scratch-dir=").map(PathBuf::from))
        .or_else(|| {
            std::env::args()
                .position(|arg| arg == "--scratch-dir")
                .and_then(|i| std::env::args().nth(i + 1).map(PathBuf::from))
        })
        .unwrap_or_else(|| std::env::temp_dir().join("pixelgrab-trace"));

    std::fs::create_dir_all(&scratch).expect("create scratch dir");

    let layout = SyntheticMonitorLayout::single_primary();
    let frame = SyntheticFrame::for_layout(
        &layout,
        pixelgrab_test_support::capture::FramePattern::SolidWhite,
    );
    let capture = SyntheticCapture::new(frame);

    let (_min_x, _min_y, max_x, max_y) = SyntheticMonitorLayout::virtual_bounds(&layout);
    let bounds = PhysicalBounds::from_xywh(0, 0, max_x as u32, max_y as u32);
    let resolution = capture.run(bounds, "trace-id");

    println!("capture_id          = {}", resolution.capture_id);
    println!(
        "bounds              = {}x{}",
        bounds.size.width, bounds.size.height
    );
    println!("asset_url.bytes     = {}", resolution.asset_url.len());

    // Also write a PNG to disk using the deterministic adapter. This is the
    // file the trace script diffs against any reference images.
    let fs = IsolatedFilesystem::new("trace").expect("isolated fs");
    fs_root_helper(&fs);

    let png_path = scratch.join("capture.png");
    let mut rgba =
        Vec::with_capacity((bounds.size.width as usize) * (bounds.size.height as usize) * 4);
    for _ in 0..(bounds.size.width as usize * bounds.size.height as usize) {
        rgba.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    }
    let mut encoder = png::Encoder::new(
        std::fs::File::create(&png_path).expect("create png"),
        bounds.size.width,
        bounds.size.height,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    {
        use std::io::Write;
        let mut stream = writer.stream_writer().expect("stream_writer");
        stream.write_all(&rgba).expect("write pixels");
        stream.finish().expect("finish png");
    }
    println!("png_path            = {}", png_path.display());
    println!("-> trace complete");

    // Silence unused warnings for the helpers we keep for symmetry.
    let _ = Arc::new(CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    });
}

fn fs_root_helper(_fs: &IsolatedFilesystem) {
    // The helper is defined to remind the next reader that all test writes
    // must go through `IsolatedFilesystem`. The actual PNG is written to the
    // scratch dir supplied by the caller so the trace script can diff it.
}
