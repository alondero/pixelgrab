//! The platform contract. Every native capability the orchestrator depends on
//! is hidden behind this trait. Real Windows implementations live in
//! `platform::windows` (introduced in tracer-02). The synthetic implementation
//! lives in `platform::synthetic` and is what the tracer-01 build uses.

use pixelgrab_contracts::{
    capture::{CaptureRequest, CaptureResolution},
    coordinate::{PhysicalBounds, PhysicalSize},
    monitor::MonitorLayout,
    PlatformResult,
};
use std::path::Path;

/// Internal capture error. Maps to `PlatformError` at the IPC boundary.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The requested capture format is not supported by this platform.
    #[error("format not supported: {0:?}")]
    UnsupportedFormat(pixelgrab_contracts::capture::CaptureFormat),
    /// The platform could not enumerate monitors.
    #[error("monitor enumeration failed: {0}")]
    MonitorEnumeration(String),
    /// The underlying capture pipeline failed.
    #[error("capture pipeline failed: {0}")]
    Pipeline(String),
    /// The capture output was rejected (e.g. zero-byte frame).
    #[error("invalid capture output: {0}")]
    InvalidOutput(String),
    /// The crop lies outside the captured framebuffer.
    #[error("crop outside framebuffer: {0}")]
    CropOutOfBounds(String),
    /// A coordinate transform produced a non-finite value.
    #[error("coordinate transform failed: {0}")]
    CoordinateTransform(String),
}

impl From<CaptureError> for pixelgrab_contracts::PlatformError {
    fn from(err: CaptureError) -> Self {
        use pixelgrab_contracts::{PlatformError, PlatformErrorKind};
        let kind = match &err {
            CaptureError::UnsupportedFormat(_) => PlatformErrorKind::Unsupported,
            CaptureError::MonitorEnumeration(_) => PlatformErrorKind::MonitorQueryFailed,
            CaptureError::Pipeline(_) => PlatformErrorKind::CaptureUnavailable,
            CaptureError::InvalidOutput(_) => PlatformErrorKind::CaptureUnavailable,
            CaptureError::CropOutOfBounds(_) => PlatformErrorKind::CoordinateTransform,
            CaptureError::CoordinateTransform(_) => PlatformErrorKind::CoordinateTransform,
        };
        PlatformError::new(kind, err.to_string())
    }
}

/// The platform contract.
///
/// All methods are synchronously callable; the orchestrator is responsible for
/// dispatching them onto the appropriate runtime. The contract is intentionally
/// `Send + Sync` so the orchestrator can be wrapped in `Arc` and shared across
/// Tauri command handlers.
pub trait PixelGrabPlatform: std::fmt::Debug + Send + Sync + std::any::Any {
    /// Enumerate the current monitor layout.
    fn monitor_layout(&self) -> PlatformResult<MonitorLayout>;

    /// Run a capture pipeline and return a `CaptureResolution`.
    fn capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution>;

    /// Write the flattened PNG bytes for the given selection. Returns the
    /// absolute path the PNG was written to.
    fn write_png(
        &self,
        capture_id: &str,
        bounds: PhysicalBounds,
        rgba: &[u8],
    ) -> PlatformResult<std::path::PathBuf>;

    /// Flatten a physical crop from the most recent capture. Returns the
    /// RGBA pixel buffer and the resulting size. The flattened buffer is
    /// the single source from which the PNG and bitmap clipboard
    /// representations are derived (per the tracer-02 acceptance criteria).
    ///
    /// The default implementation rejects the call with `Unsupported` so the
    /// synthetic adapter (which has no captured pixels) does not need to
    /// override it; Windows replaces it with a frozen-framebuffer read.
    fn flatten_crop(
        &self,
        _capture_id: &str,
        _crop: PhysicalBounds,
    ) -> PlatformResult<(Vec<u8>, PhysicalSize)> {
        Err(pixelgrab_contracts::PlatformError::new(
            pixelgrab_contracts::PlatformErrorKind::Unsupported,
            "platform does not expose a frozen framebuffer",
        ))
    }

    /// Publish the flattened crop to the system clipboard as both PNG and
    /// a bitmap-compatible representation. Returns Ok when the platform
    /// does not own a clipboard (synthetic adapter).
    fn publish_clipboard(
        &self,
        _capture_id: &str,
        _rgba: &[u8],
        _size: PhysicalSize,
    ) -> PlatformResult<()> {
        Ok(())
    }

    /// Publish an existing on-disk PNG to the system clipboard. Tracer 08
    /// uses this for the shelf card Copy quick action: the cached PNG is
    /// re-published to the clipboard without re-flattening the crop.
    ///
    /// The default implementation reads the PNG bytes, decodes them via
    /// the `png` crate, and forwards the resulting RGBA buffer to
    /// `publish_clipboard`. Platforms that own a native clipboard (e.g.
    /// Windows via `arboard`) can override this to write the PNG bytes
    /// directly.
    fn publish_png_clipboard(&self, png_path: &Path) -> PlatformResult<()> {
        let bytes = std::fs::read(png_path).map_err(|err| {
            pixelgrab_contracts::PlatformError::new(
                pixelgrab_contracts::PlatformErrorKind::Io,
                format!("read png for clipboard: {err}"),
            )
        })?;
        let decoder = png::Decoder::new(bytes.as_slice());
        let mut reader = decoder.read_info().map_err(|e| {
            pixelgrab_contracts::PlatformError::new(
                pixelgrab_contracts::PlatformErrorKind::InvalidPayload,
                format!("decode png: {e}"),
            )
        })?;
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| {
            pixelgrab_contracts::PlatformError::new(
                pixelgrab_contracts::PlatformErrorKind::InvalidPayload,
                format!("read png frame: {e}"),
            )
        })?;
        buf.truncate(info.buffer_size());
        let size = PhysicalSize::new(info.width, info.height);
        // Capture id is the file stem; it has no meaning to the
        // synthetic adapter but is kept for parity with
        // `publish_clipboard`.
        let capture_id = png_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("cached");
        self.publish_clipboard(capture_id, &buf, size)
    }
}
