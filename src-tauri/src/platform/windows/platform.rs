//! `WindowsPlatform` — the Rust-side implementation of `PixelGrabPlatform`
//! backed by the `xcap`-driven `CaptureEngine` and a single-source
//! flattened crop pipeline.
//!
//! The platform owns:
//!
//! - a `CaptureEngine` for monitor enumeration, capture, and frozen-frame
//!   retention;
//! - the cache root where flattened PNGs are written for shelf persistence;
//! - the clipboard publish path (PNG + bitmap-compatible CF_DIB).
//!
//! All clipboard bytes are derived from a single flattened RGBA buffer so
//! the PNG and the bitmap never disagree about pixel values.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use pixelgrab_contracts::{
    capture::{CaptureRequest, CaptureResolution},
    coordinate::{PhysicalBounds, PhysicalSize},
    monitor::MonitorLayout,
    PlatformError, PlatformErrorKind, PlatformResult,
};

use super::super::contract::PixelGrabPlatform;
use super::capture::{encode_png, CaptureEngine};

/// Concrete Windows implementation of `PixelGrabPlatform`.
#[derive(Debug, Clone)]
pub struct WindowsPlatform {
    inner: Arc<WindowsPlatformState>,
}

#[derive(Debug)]
struct WindowsPlatformState {
    engine: CaptureEngine,
    cache_root: Mutex<Option<PathBuf>>,
}

impl WindowsPlatform {
    /// Build a new platform. The capture engine is uninitialised until the
    /// first `capture()` or `monitor_layout()` call queries the OS.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(WindowsPlatformState {
                engine: CaptureEngine::new(),
                cache_root: Mutex::new(None),
            }),
        }
    }

    /// Build a platform with an explicit cache root for flattened PNGs.
    pub fn with_cache_root(cache_root: PathBuf) -> Self {
        let platform = Self::new();
        *platform.inner.cache_root.lock() = Some(cache_root);
        platform
    }

    /// Borrow the underlying capture engine. Used by the session
    /// orchestrator to wire diagnostics and overlay visibility timestamps
    /// without going through the platform trait.
    pub fn engine(&self) -> CaptureEngine {
        self.inner.engine.clone()
    }

    /// Replace the cache root. The commit pipeline writes flattened PNGs
    /// here when the user retains the capture on the shelf. The capture
    /// engine also persists freeze frames under `<root>/frames` so the
    /// overlay loads them through the asset protocol instead of inline
    /// base64 (issue #63).
    pub fn set_cache_root(&self, cache_root: PathBuf) {
        *self.inner.cache_root.lock() = Some(cache_root.clone());
        // The asset layer appends `frames/` itself (issue #63).
        self.inner.engine.set_frame_cache_root(Some(cache_root));
    }

    /// Read the configured cache root. Returns None if unset.
    pub fn cache_root(&self) -> Option<PathBuf> {
        self.inner.cache_root.lock().clone()
    }
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl PixelGrabPlatform for WindowsPlatform {
    fn monitor_layout(&self) -> PlatformResult<MonitorLayout> {
        self.inner.engine.monitor_layout()
    }

    fn invalidate_layout(&self) {
        self.inner.engine.invalidate_layout();
    }

    fn release_capture(&self) {
        self.inner.engine.clear();
    }

    fn cursor_position(&self) -> Option<pixelgrab_contracts::coordinate::PhysicalPoint> {
        super::work_area::ffi::query_cursor_position()
            .map(|(x, y)| pixelgrab_contracts::coordinate::PhysicalPoint::new(x, y))
    }

    fn capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution> {
        self.inner.engine.capture(request)
    }

    fn write_png(
        &self,
        capture_id: &str,
        bounds: PhysicalBounds,
        rgba: &[u8],
    ) -> PlatformResult<PathBuf> {
        let root = self.inner.cache_root.lock().clone().ok_or_else(|| {
            PlatformError::new(PlatformErrorKind::Io, "windows cache root not configured")
        })?;
        let bytes = encode_png(rgba, bounds.size)?;
        let path = root.join(format!("{capture_id}.png"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &bytes)?;
        Ok(path)
    }

    fn flatten_crop(
        &self,
        capture_id: &str,
        crop: PhysicalBounds,
    ) -> PlatformResult<(Vec<u8>, PhysicalSize)> {
        let frame = self.inner.engine.frozen().ok_or_else(|| {
            PlatformError::new(
                PlatformErrorKind::InvalidSessionState,
                "no frozen frame available for the requested capture_id",
            )
        })?;
        if frame.capture_id != capture_id {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                "capture_id does not match the frozen framebuffer",
            ));
        }
        let rgba = frame.crop(&crop)?;
        let size = crop.size;
        // Save As is non-terminal and may be cancelled or repeated. The
        // session owns the frozen source until commit or cancellation.
        Ok((rgba, size))
    }

    fn publish_clipboard(
        &self,
        capture_id: &str,
        rgba: &[u8],
        size: PhysicalSize,
    ) -> PlatformResult<()> {
        publish_to_clipboard(capture_id, rgba, size)
    }

    fn start_drag(
        &self,
        request: &pixelgrab_contracts::drag::DragRequest,
    ) -> PlatformResult<pixelgrab_contracts::drag::DragResult> {
        super::drag::start_drag(request)
    }
}

/// Push the flattened crop to the system clipboard. The single
/// representation we publish is a Windows CF_DIB bitmap (top-down BGRA)
/// derived from the same flattened RGBA buffer as the on-disk PNG. The
/// bitmap is the bitmap-compatible form asked for by the tracer-02
/// acceptance criteria; the on-disk PNG (written by `write_png`) is the
/// lossless copy consumed by apps that prefer to read the file directly.
///
/// `capture_id` is retained in the signature so future revisions can
/// attach the PNG bytes to the OS as a custom format without changing the
/// platform contract.
fn publish_to_clipboard(_capture_id: &str, rgba: &[u8], size: PhysicalSize) -> PlatformResult<()> {
    use arboard::Clipboard;
    let image = clipboard_image(rgba, size);
    let mut clipboard = Clipboard::new().map_err(|err| {
        PlatformError::new(
            PlatformErrorKind::CaptureUnavailable,
            format!("clipboard unavailable: {err}"),
        )
    })?;
    clipboard.set_image(image).map_err(|err| {
        PlatformError::new(
            PlatformErrorKind::Io,
            format!("clipboard image write failed: {err}"),
        )
    })?;
    Ok(())
}

// arboard accepts RGBA and converts to Windows bitmap order itself.
fn clipboard_image(rgba: &[u8], size: PhysicalSize) -> arboard::ImageData<'_> {
    arboard::ImageData {
        width: size.width as usize,
        height: size.height as usize,
        bytes: std::borrow::Cow::Borrowed(rgba),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_adapter_passes_rgba_to_arboard() {
        let rgba = vec![10, 20, 30, 40];
        let image = clipboard_image(&rgba, PhysicalSize::new(1, 1));
        assert_eq!(image.bytes.as_ref(), &[10, 20, 30, 40]);
    }

    #[test]
    fn default_constructor_has_no_cache_root() {
        let platform = WindowsPlatform::new();
        assert!(platform.cache_root().is_none());
    }

    #[test]
    fn set_cache_root_round_trips() {
        let platform = WindowsPlatform::new();
        platform.set_cache_root(std::env::temp_dir().join("pixelgrab-test"));
        assert!(platform.cache_root().is_some());
    }
}
