//! Synthetic capture: a deterministic framebuffer that never contains real
//! desktop content.

use pixelgrab_contracts::{
    capture::{CaptureFormat, CaptureResolution},
    coordinate::{PhysicalBounds, PhysicalSize},
    monitor::MonitorLayout,
};

/// A deterministic framebuffer indexing helper. The bytes themselves are
/// lazy: computed only when materialised by the PNG encoder.
#[derive(Debug, Clone)]
pub struct SyntheticFrame {
    /// Physical-pixel size.
    pub size: PhysicalSize,
    /// Per-pixel RGBA pattern. The pattern is a deterministic gradient with
    /// a corner watermark so tests can verify orientation without leaking
    /// real desktop content.
    pub pattern: FramePattern,
}

/// Visual pattern used by the synthetic frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePattern {
    /// Top-left solid colour regions, one per monitor.
    SolidPerMonitor,
    /// Diagonal gradient with the PixelGrab watermark.
    GradientWithWatermark,
    /// Plain solid colour. Used for golden-image comparison.
    SolidWhite,
}

impl SyntheticFrame {
    /// Render a virtual-desktop frame for the given layout + pattern.
    pub fn for_layout(layout: &MonitorLayout, pattern: FramePattern) -> Self {
        let (min_x, min_y, max_x, max_y) =
            super::layout::SyntheticMonitorLayout::virtual_bounds(layout);
        let size = PhysicalSize::new((max_x - min_x) as u32, (max_y - min_y) as u32);
        Self { size, pattern }
    }

    /// Encode the frame to PNG bytes. The bytes are deterministic - tests
    /// may hash them.
    pub fn to_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let width = self.size.width;
        let height = self.size.height;
        let mut buf = Vec::with_capacity(((width as usize) * (height as usize) * 4) + 1024);
        {
            let mut encoder = png::Encoder::new(&mut buf, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            {
                use std::io::Write;
                let mut stream = writer.stream_writer()?;
                for y in 0..height {
                    for x in 0..width {
                        let px = self.pixel(x, y);
                        stream.write_all(&px)?;
                    }
                }
                stream.finish()?;
            }
            // writer is dropped here, releasing the borrow on `buf`.
        }
        Ok(buf)
    }

    /// Compute a single pixel for the given coordinate. Pure function.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        match self.pattern {
            FramePattern::SolidWhite => [255, 255, 255, 255],
            FramePattern::SolidPerMonitor => {
                let r = ((x / 64) & 0xFF) as u8;
                let g = ((y / 64) & 0xFF) as u8;
                let b = (((x ^ y) / 32) & 0xFF) as u8;
                [r, g, b, 255]
            }
            FramePattern::GradientWithWatermark => {
                let r = (x & 0xFF) as u8;
                let g = (y & 0xFF) as u8;
                let b = (((x + y) >> 1) & 0xFF) as u8;
                let in_watermark =
                    (40..120).contains(&x) && (40..120).contains(&y) && (x + y) % 32 < 16;
                if in_watermark {
                    [0, 0, 0, 255]
                } else {
                    [r, g, b, 255]
                }
            }
        }
    }
}

/// The synthetic capture pipeline. Returns a `CaptureResolution` with a
/// `data:` URL so the WebView can load the PNG bytes without any real disk
/// or asset-protocol setup.
#[derive(Debug, Clone)]
pub struct SyntheticCapture {
    frame: SyntheticFrame,
}

impl SyntheticCapture {
    /// Build a synthetic capture from a prepared frame.
    pub fn new(frame: SyntheticFrame) -> Self {
        Self { frame }
    }

    /// Run the capture and return a deterministic `CaptureResolution`.
    pub fn run(&self, bounds: PhysicalBounds, capture_id: &str) -> CaptureResolution {
        // The asset URL embeds a base64-encoded PNG so no filesystem is
        // required for the synthetic path. Real desktop builds swap this for
        // an `asset://localhost/...` URL.
        let png = self.frame.to_png().expect("synthetic png encode");
        let url = format!("data:image/png;base64,{}", base64_encode(&png));
        CaptureResolution {
            format: CaptureFormat::VirtualDesktop,
            bounds,
            asset_url: url,
            capture_id: capture_id.to_string(),
            captured_at_ms: 1_700_000_000_000,
        }
    }
}

/// Minimal RFC 4648 base64 encoder implementation. We avoid pulling in a
/// crate just for this; the encoder is trivial and only used in tests.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let b = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(ALPHABET[((b >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((b >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((b >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(b & 0x3F) as usize] as char);
        i += 3;
    }
    match input.len() - i {
        1 => {
            let b = (input[i] as u32) << 16;
            out.push(ALPHABET[((b >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
            out.push(ALPHABET[((b >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3F) as usize] as char);
            out.push(ALPHABET[((b >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}
