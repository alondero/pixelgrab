//! Synthetic platform implementation. Drives the orchestrator end-to-end
//! without any Windows/native dependencies. Used by tracer-01 and by the
//! integration tests.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use pixelgrab_contracts::{
    capture::{CaptureFormat, CaptureRequest, CaptureResolution},
    coordinate::{PhysicalBounds, PhysicalSize},
    monitor::MonitorLayout,
    PlatformError, PlatformErrorKind, PlatformResult,
};
use pixelgrab_test_support::{layout::SyntheticMonitorLayout, SyntheticCapture, SyntheticFrame};
use uuid::Uuid;

use super::contract::PixelGrabPlatform;

/// The synthetic platform. Holds the test layout, the synthetic capture, and
/// a path to the isolated filesystem root under which PNGs are written.
#[derive(Debug, Clone)]
pub struct SyntheticPlatform {
    inner: Arc<SyntheticPlatformState>,
}

#[derive(Debug)]
struct SyntheticPlatformState {
    layout: Mutex<MonitorLayout>,
    capture: SyntheticCapture,
    cache_root: Mutex<Option<PathBuf>>,
}

impl SyntheticPlatform {
    /// Build a new synthetic platform with the default single-monitor layout.
    pub fn new() -> Self {
        let layout = SyntheticMonitorLayout::single_primary();
        let (_, _, max_x, max_y) = SyntheticMonitorLayout::virtual_bounds(&layout);
        let frame = SyntheticFrame {
            size: PhysicalSize::new(max_x as u32, max_y as u32),
            pattern: pixelgrab_test_support::capture::FramePattern::GradientWithWatermark,
        };
        Self {
            inner: Arc::new(SyntheticPlatformState {
                layout: Mutex::new(layout),
                capture: SyntheticCapture::new(frame),
                cache_root: Mutex::new(None),
            }),
        }
    }

    /// Build a synthetic platform with a custom layout and pattern.
    pub fn with_layout(
        layout: MonitorLayout,
        pattern: pixelgrab_test_support::capture::FramePattern,
    ) -> Self {
        let (_, _, max_x, max_y) = SyntheticMonitorLayout::virtual_bounds(&layout);
        let frame = SyntheticFrame {
            size: PhysicalSize::new(max_x as u32, max_y as u32),
            pattern,
        };
        Self {
            inner: Arc::new(SyntheticPlatformState {
                layout: Mutex::new(layout),
                capture: SyntheticCapture::new(frame),
                cache_root: Mutex::new(None),
            }),
        }
    }

    /// Set the root directory where PNGs get written. Calls without a root
    /// return an Io error.
    pub fn set_cache_root(&self, root: PathBuf) {
        *self.inner.cache_root.lock() = Some(root);
    }

    /// Replace the monitor layout. Used by tests that need to simulate
    /// display changes.
    pub fn set_layout(&self, layout: MonitorLayout) {
        *self.inner.layout.lock() = layout;
    }

    /// Try to downcast an `Arc<dyn PixelGrabPlatform>` to a concrete
    /// `SyntheticPlatform`. Returns `None` if the underlying implementation
    /// is not synthetic.
    ///
    /// Implemented via `TypeId` comparison so the result is safe.
    pub fn downcast(platform: Arc<dyn PixelGrabPlatform>) -> Option<Arc<Self>> {
        use std::any::TypeId;
        if platform.type_id() == TypeId::of::<SyntheticPlatform>() {
            // SAFETY: The `TypeId` check above guarantees the inner value is
            // a `SyntheticPlatform`. We round-trip through a raw pointer to
            // recover the concrete `Arc`. The original `Arc` is dropped by
            // the caller; we transfer ownership of the strong count.
            let raw = Arc::into_raw(platform) as *const SyntheticPlatform;
            let typed = unsafe { Arc::from_raw(raw) };
            Some(typed)
        } else {
            None
        }
    }
}

impl Default for SyntheticPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl PixelGrabPlatform for SyntheticPlatform {
    fn monitor_layout(&self) -> PlatformResult<MonitorLayout> {
        Ok(self.inner.layout.lock().clone())
    }

    fn capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution> {
        let layout = self.inner.layout.lock().clone();
        let (min_x, min_y, max_x, max_y) = SyntheticMonitorLayout::virtual_bounds(&layout);
        let bounds = match request.format {
            CaptureFormat::VirtualDesktop => PhysicalBounds::from_xywh(
                min_x,
                min_y,
                (max_x - min_x) as u32,
                (max_y - min_y) as u32,
            ),
            CaptureFormat::SingleMonitor => {
                let id = request.monitor_id.as_deref().ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorKind::InvalidPayload,
                        "SingleMonitor format requires monitor_id",
                    )
                })?;
                layout
                    .monitors
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| m.bounds)
                    .ok_or_else(|| {
                        PlatformError::new(
                            PlatformErrorKind::MonitorQueryFailed,
                            format!("monitor id not found: {id}"),
                        )
                    })?
            }
            CaptureFormat::PhysicalRegion => request.region.ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorKind::InvalidPayload,
                    "PhysicalRegion format requires region",
                )
            })?,
        };
        let capture_id = Uuid::new_v4().to_string();
        Ok(self.inner.capture.run(bounds, &capture_id))
    }

    fn write_png(
        &self,
        capture_id: &str,
        bounds: PhysicalBounds,
        rgba: &[u8],
    ) -> PlatformResult<PathBuf> {
        let root = self.inner.cache_root.lock().clone().ok_or_else(|| {
            PlatformError::new(PlatformErrorKind::Io, "synthetic cache root not configured")
        })?;
        let path = root.join(format!("{capture_id}.png"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let width = bounds.size.width;
        let height = bounds.size.height;
        if rgba.len() != (width as usize) * (height as usize) * 4 {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                format!(
                    "rgba buffer length {} does not match {}x{}x4",
                    rgba.len(),
                    width,
                    height
                ),
            ));
        }
        let file = std::fs::File::create(&path)?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| PlatformError::new(PlatformErrorKind::Io, e.to_string()))?;
        {
            use std::io::Write;
            let mut stream = writer
                .stream_writer()
                .map_err(|e| PlatformError::new(PlatformErrorKind::Io, e.to_string()))?;
            stream.write_all(rgba)?;
            stream
                .finish()
                .map_err(|e| PlatformError::new(PlatformErrorKind::Io, e.to_string()))?;
        }
        Ok(path)
    }

    fn flatten_crop(
        &self,
        capture_id: &str,
        crop: PhysicalBounds,
    ) -> PlatformResult<(Vec<u8>, PhysicalSize)> {
        // The synthetic adapter has no frozen framebuffer; produce a
        // deterministic gradient identical to the commit pipeline's tracer-01
        // behaviour so the IPC contract stays exercised end-to-end.
        if crop.size.width == 0 || crop.size.height == 0 {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                "synthetic flatten_crop: zero-sized crop",
            ));
        }
        let width = crop.size.width as usize;
        let height = crop.size.height as usize;
        let mut rgba = vec![0u8; width * height * 4];
        for (i, chunk) in rgba.chunks_exact_mut(4).enumerate() {
            let x = (i % width) as u32;
            let y = (i / width) as u32;
            chunk[0] = (x & 0xFF) as u8;
            chunk[1] = (y & 0xFF) as u8;
            chunk[2] = (((x ^ y) >> 1) & 0xFF) as u8;
            chunk[3] = 0xFF;
        }
        let size = crop.size;
        // `capture_id` is consumed for parity with the Windows contract;
        // the synthetic path doesn't key the buffer by id.
        let _ = capture_id;
        Ok((rgba, size))
    }
}
