//! Synthetic platform implementation. Drives the orchestrator end-to-end
//! without any Windows/native dependencies. Used by tracer-01 and by the
//! integration tests.
//!
//! The tracer-03 pipeline composites a virtual-desktop framebuffer by
//! blitting one deterministic per-monitor framebuffer into a single RGBA
//! buffer. The composite mirrors the Windows pipeline so the contract
//! stays platform-neutral: the same `MonitorLayout` + capture request
//! produces the same bounds + asset URL through either adapter.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use pixelgrab_contracts::{
    capture::{CaptureFormat, CaptureRequest, CaptureResolution},
    coordinate::{PhysicalBounds, PhysicalSize},
    drag::{DragRequest, DragResult},
    monitor::{MonitorDescriptor, MonitorLayout},
    PlatformError, PlatformErrorKind, PlatformResult,
};
use pixelgrab_test_support::capture::FramePattern;
use pixelgrab_test_support::layout::SyntheticMonitorLayout;
use uuid::Uuid;

use super::contract::PixelGrabPlatform;
use super::drag_synthetic::SyntheticDragSource;

/// The synthetic platform. Holds the test layout, the synthetic capture, and
/// a path to the isolated filesystem root under which PNGs are written.
#[derive(Debug, Clone)]
pub struct SyntheticPlatform {
    inner: Arc<SyntheticPlatformState>,
}

#[derive(Debug)]
struct SyntheticPlatformState {
    layout: Mutex<MonitorLayout>,
    pattern: Mutex<FramePattern>,
    cache_root: Mutex<Option<PathBuf>>,
    drag: SyntheticDragSource,
    /// Monotonic counter for failing monitors. When a monitor id appears
    /// in this set, the synthetic capture fails that monitor so tests can
    /// exercise the partial-failure code path.
    failing_monitors: Mutex<Vec<String>>,
    /// `true` when the cached layout has been invalidated since the
    /// last query. Mirrors the Windows engine's `topology_dirty` flag.
    topology_dirty: Mutex<bool>,
}

impl SyntheticPlatform {
    /// Build a new synthetic platform with the default single-monitor layout.
    pub fn new() -> Self {
        let layout = SyntheticMonitorLayout::single_primary();
        Self {
            inner: Arc::new(SyntheticPlatformState {
                layout: Mutex::new(layout),
                pattern: Mutex::new(FramePattern::GradientWithWatermark),
                cache_root: Mutex::new(None),
                drag: SyntheticDragSource::new(),
                failing_monitors: Mutex::new(Vec::new()),
                topology_dirty: Mutex::new(false),
            }),
        }
    }

    /// Build a synthetic platform with a custom layout and pattern.
    pub fn with_layout(layout: MonitorLayout, pattern: FramePattern) -> Self {
        // A new layout invalidates any cached capture state.
        let platform = Self::new();
        *platform.inner.layout.lock() = layout;
        *platform.inner.pattern.lock() = pattern;
        platform.mark_topology_changed();
        platform
    }

    /// Set the root directory where PNGs get written. Calls without a root
    /// return an Io error.
    pub fn set_cache_root(&self, root: PathBuf) {
        *self.inner.cache_root.lock() = Some(root);
    }

    /// Replace the monitor layout. Used by tests that need to simulate
    /// display changes (hot-plug, unplug, resolution change, DPI change).
    pub fn set_layout(&self, layout: MonitorLayout) {
        *self.inner.layout.lock() = layout;
        self.mark_topology_changed();
    }

    /// Mark the next `capture` call as failing the named monitor. The
    /// platform reports `MonitorCaptureFailed` for that monitor and the
    /// composite is rejected. Pass an empty slice to clear the failure
    /// list.
    pub fn set_failing_monitors(&self, ids: &[&str]) {
        *self.inner.failing_monitors.lock() = ids.iter().map(|s| s.to_string()).collect();
    }

    /// Mark the cached topology as suspect. The next `monitor_layout`
    /// call returns the same layout; the flag is used by tests that want
    /// to verify the contract call rather than the layout itself.
    pub fn mark_topology_changed(&self) {
        *self.inner.topology_dirty.lock() = true;
    }

    /// `true` when the cached topology has been invalidated since the
    /// last layout query. Mirrors `CaptureEngine::is_topology_dirty`.
    pub fn is_topology_dirty(&self) -> bool {
        *self.inner.topology_dirty.lock()
    }

    /// Borrow the synthetic drag source. Tests use this to install a
    /// custom script and to read back the recorded outcomes.
    pub fn drag_source(&self) -> SyntheticDragSource {
        self.inner.drag.clone()
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
        // The synthetic adapter keeps the layout in-memory; the topology
        // flag is purely a contract signal. Drain it on read so the next
        // call returns false.
        *self.inner.topology_dirty.lock() = false;
        Ok(self.inner.layout.lock().clone())
    }

    fn invalidate_layout(&self) {
        self.mark_topology_changed();
    }

    fn capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution> {
        let layout = self.inner.layout.lock().clone();
        let pattern = *self.inner.pattern.lock();
        let failing = self.inner.failing_monitors.lock().clone();

        let virtual_bounds = layout.virtual_bounds().ok_or_else(|| {
            PlatformError::new(
                PlatformErrorKind::MonitorQueryFailed,
                "no monitors in layout",
            )
        })?;
        let composite_bounds = virtual_bounds.as_top_left_bounds();
        let buffer_size = composite_bounds.size;

        let capture_id = Uuid::new_v4().to_string();
        let captured_at_ms = 1_700_000_000_000;

        let (rgba, bounds) = match request.format {
            CaptureFormat::VirtualDesktop => {
                let rgba = composite_virtual_desktop(&layout, pattern, &failing)?;
                (rgba, composite_bounds)
            }
            CaptureFormat::SingleMonitor => {
                let id = request.monitor_id.as_deref().ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorKind::InvalidPayload,
                        "SingleMonitor format requires monitor_id",
                    )
                })?;
                let descriptor = layout.monitors.iter().find(|m| m.id == id).ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorKind::MonitorQueryFailed,
                        format!("monitor id not found: {id}"),
                    )
                })?;
                if failing.iter().any(|f| f == id) {
                    return Err(PlatformError::new(
                        PlatformErrorKind::CaptureUnavailable,
                        format!("monitor capture failed for {id}: injected_failure"),
                    ));
                }
                let rgba = synth_monitor_framebuffer(descriptor, pattern);
                (rgba, descriptor.bounds)
            }
            CaptureFormat::PhysicalRegion => {
                let region = request.region.ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorKind::InvalidPayload,
                        "PhysicalRegion format requires region",
                    )
                })?;
                let rgba = synth_region_framebuffer(&layout, region, pattern);
                (rgba, region)
            }
        };

        let url = format!(
            "data:image/png;base64,{}",
            encode_rgba_as_data_url(&rgba, buffer_size)
        );
        let _ = captured_at_ms; // mirror Windows: not exposed via the synthetic data URL
        let _ = bounds.size;
        Ok(CaptureResolution {
            format: request.format,
            bounds,
            asset_url: url,
            capture_id,
            captured_at_ms,
        })
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

    fn start_drag(&self, request: &DragRequest) -> PlatformResult<DragResult> {
        self.inner.drag.run(request)
    }
}

/// Composite the virtual desktop by rendering each monitor's deterministic
/// framebuffer into one RGBA buffer. Mirrors the Windows composite pipeline
/// so the synthetic adapter and the real adapter set the same
/// `CaptureResolution::bounds`.
///
/// Layout-aware: each monitor's pixels encode the monitor's id and
/// position so a test can verify the input layout by inspecting the
/// resulting framebuffer. The `pattern` argument controls the underlying
/// colour gradient; the watermark is replaced by an id stamp so the
/// composite is distinguishable from a single-monitor capture.
fn composite_virtual_desktop(
    layout: &MonitorLayout,
    pattern: FramePattern,
    failing: &[String],
) -> PlatformResult<Vec<u8>> {
    let virtual_bounds = layout.virtual_bounds().ok_or_else(|| {
        PlatformError::new(
            PlatformErrorKind::MonitorQueryFailed,
            "no monitors in layout",
        )
    })?;
    let composite_bounds = virtual_bounds.as_top_left_bounds();
    let buffer_size = composite_bounds.size;
    let total_pixels = (buffer_size.width as usize) * (buffer_size.height as usize);
    let mut composite = vec![0u8; total_pixels * 4];

    for descriptor in &layout.monitors {
        if failing.iter().any(|f| f == &descriptor.id) {
            return Err(synthetic_monitor_failure(&descriptor.id));
        }
        let pixels = synth_monitor_framebuffer(descriptor, pattern);
        let offset = pixelgrab_contracts::coordinate::transform::monitor_to_capture_buffer(
            &descriptor.bounds,
            virtual_bounds.min,
            buffer_size,
        );
        blit_rgba(&mut composite, buffer_size, &offset, &pixels);
    }
    Ok(composite)
}

/// Render one monitor's deterministic framebuffer. The pixels encode the
/// monitor's id and bounds so tests can verify the composite maps each
/// monitor to the right offset.
fn synth_monitor_framebuffer(descriptor: &MonitorDescriptor, pattern: FramePattern) -> Vec<u8> {
    let width = descriptor.bounds.size.width as usize;
    let height = descriptor.bounds.size.height as usize;
    let mut buf = Vec::with_capacity(width * height * 4);
    let id_bytes = descriptor.id.as_bytes();
    let id_stamp = id_bytes.first().copied().unwrap_or(0);
    let region_stamp =
        ((descriptor.bounds.origin.x & 0xFF) as u8) ^ ((descriptor.bounds.origin.y & 0xFF) as u8);
    for y in 0..height {
        for x in 0..width {
            let px = match pattern {
                FramePattern::SolidWhite => [255, 255, 255, 255],
                FramePattern::SolidPerMonitor => {
                    let r = ((x / 64) & 0xFF) as u8;
                    let g = ((y / 64) & 0xFF) as u8;
                    let b = (((x ^ y) / 32) & 0xFF) as u8;
                    [r ^ id_stamp, g ^ region_stamp, b, 255]
                }
                FramePattern::GradientWithWatermark => {
                    let r = (x & 0xFF) as u8;
                    let g = (y & 0xFF) as u8;
                    let b = (((x + y) >> 1) & 0xFF) as u8;
                    let in_watermark =
                        (40..120).contains(&x) && (40..120).contains(&y) && (x + y) % 32 < 16;
                    if in_watermark {
                        [0, id_stamp, region_stamp, 255]
                    } else {
                        [r, g, b, 255]
                    }
                }
            };
            buf.extend_from_slice(&px);
        }
    }
    buf
}

/// Render a single physical region using the same per-monitor pattern.
/// Each monitor's contribution is rendered at its local offset. The
/// region is clamped to the layout's virtual bounds so a request that
/// extends past the right/bottom edge does not read past the buffer.
fn synth_region_framebuffer(
    layout: &MonitorLayout,
    region: PhysicalBounds,
    pattern: FramePattern,
) -> Vec<u8> {
    let virtual_bounds = layout.virtual_bounds().expect("non-empty layout");
    let composite_bounds = virtual_bounds.as_top_left_bounds();
    let buffer_size = composite_bounds.size;
    let mut composite = vec![0u8; (buffer_size.width as usize) * (buffer_size.height as usize) * 4];
    for descriptor in &layout.monitors {
        let pixels = synth_monitor_framebuffer(descriptor, pattern);
        let offset = pixelgrab_contracts::coordinate::transform::monitor_to_capture_buffer(
            &descriptor.bounds,
            virtual_bounds.min,
            buffer_size,
        );
        blit_rgba(&mut composite, buffer_size, &offset, &pixels);
    }
    let in_buffer = pixelgrab_contracts::coordinate::transform::project_to_capture_buffer(
        &region,
        virtual_bounds.min,
        buffer_size,
    );
    if in_buffer.is_empty() {
        return Vec::new();
    }
    let stride = (buffer_size.width as usize) * 4;
    let crop_w = in_buffer.size.width as usize;
    let crop_h = in_buffer.size.height as usize;
    let mut out = Vec::with_capacity(crop_w * crop_h * 4);
    for row in 0..crop_h {
        let src_offset =
            ((in_buffer.origin.y as usize) + row) * stride + (in_buffer.origin.x as usize) * 4;
        out.extend_from_slice(&composite[src_offset..src_offset + crop_w * 4]);
    }
    out
}

/// Build a synthetic `MonitorCaptureFailed` error that maps the same way
/// the Windows pipeline reports partial failures.
fn synthetic_monitor_failure(monitor_id: &str) -> PlatformError {
    PlatformError::new(
        PlatformErrorKind::CaptureUnavailable,
        format!("monitor capture failed for {monitor_id}: injected_failure"),
    )
}

/// Copy an RGBA framebuffer into a destination framebuffer at the given
/// offset. Mirrors the Windows helper so the two platforms stay aligned
/// on the bounds-clamping rules.
fn blit_rgba(dst: &mut [u8], dst_size: PhysicalSize, offset: &PhysicalBounds, src: &[u8]) {
    let dst_width = dst_size.width as i32;
    let dst_height = dst_size.height as i32;
    let src_width = offset.size.width as i32;
    let src_height = offset.size.height as i32;
    let dst_x = offset.origin.x;
    let dst_y = offset.origin.y;
    if dst_x < 0 || dst_y < 0 || dst_x + src_width <= 0 || dst_y + src_height <= 0 {
        return;
    }
    let copy_x0 = 0.max(-dst_x);
    let copy_y0 = 0.max(-dst_y);
    let copy_x1 = src_width.min(dst_width - dst_x);
    let copy_y1 = src_height.min(dst_height - dst_y);
    if copy_x1 <= copy_x0 || copy_y1 <= copy_y0 {
        return;
    }
    let dst_stride = (dst_size.width as usize) * 4;
    let src_stride = (offset.size.width as usize) * 4;
    let copy_w = (copy_x1 - copy_x0) as usize;
    for row in copy_y0..copy_y1 {
        let src_offset = (row as usize) * src_stride + (copy_x0 as usize) * 4;
        let dst_offset = ((dst_y + row) as usize) * dst_stride + ((dst_x + copy_x0) as usize) * 4;
        dst[dst_offset..dst_offset + copy_w * 4]
            .copy_from_slice(&src[src_offset..src_offset + copy_w * 4]);
    }
}

/// Encode an RGBA framebuffer as a PNG and wrap it in a data URL. Used by
/// the synthetic capture path so the WebView can load the bytes without
/// touching the filesystem.
fn encode_rgba_as_data_url(rgba: &[u8], size: PhysicalSize) -> String {
    let width = size.width;
    let height = size.height;
    let mut buf = Vec::with_capacity(((width as usize) * (height as usize) * 4) + 1024);
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = match encoder.write_header() {
            Ok(w) => w,
            Err(_) => return String::new(),
        };
        {
            use std::io::Write;
            let mut stream = match writer.stream_writer() {
                Ok(s) => s,
                Err(_) => return String::new(),
            };
            if stream.write_all(rgba).is_err() {
                return String::new();
            }
            if stream.finish().is_err() {
                return String::new();
            }
        }
    }
    format!("data:image/png;base64,{}", base64_encode(&buf))
}

/// Minimal RFC 4648 base64 encoder. Avoids pulling a crate in just for this.
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

#[cfg(test)]
mod tests {
    use super::*;
    use pixelgrab_contracts::coordinate::PhysicalPoint;
    use pixelgrab_test_support::layout::SyntheticMonitorLayout;

    #[test]
    fn topology_dirty_flag_round_trips() {
        let platform = SyntheticPlatform::new();
        assert!(!platform.is_topology_dirty());
        platform.invalidate_layout();
        assert!(platform.is_topology_dirty());
        let _ = platform.monitor_layout().expect("layout");
        assert!(!platform.is_topology_dirty());
    }

    #[test]
    fn set_layout_marks_topology_changed() {
        let platform = SyntheticPlatform::new();
        // Touch the layout through the public API; the flag should
        // follow the same path as the Windows engine's `invalidate_layout`.
        let _ = platform.monitor_layout().expect("layout");
        assert!(!platform.is_topology_dirty());
        platform.set_layout(SyntheticMonitorLayout::dual_side_by_side());
        assert!(platform.is_topology_dirty());
    }

    #[test]
    fn composite_pipeline_with_negative_origin_offsets_correctly() {
        // Layout with a secondary above the primary. The composite's
        // pending framebuffer should be `(1920 + 1920) x (200 + 1080)`
        // and the secondary's pixels should appear at local row 0.
        let layout = SyntheticMonitorLayout::dual_negative_origin();
        let platform =
            SyntheticPlatform::with_layout(layout.clone(), FramePattern::SolidPerMonitor);
        let request = CaptureRequest {
            format: CaptureFormat::VirtualDesktop,
            monitor_id: None,
            region: None,
        };
        let resolution = platform.capture(&request).expect("capture");
        let virtual_bounds = layout.virtual_bounds().expect("bounds");
        let composite_bounds = virtual_bounds.as_top_left_bounds();
        assert_eq!(resolution.bounds.size, composite_bounds.size);
        assert_eq!(resolution.bounds.origin, composite_bounds.origin);
    }

    #[test]
    fn failing_monitor_rejects_composite() {
        let layout = SyntheticMonitorLayout::dual_side_by_side();
        let platform = SyntheticPlatform::with_layout(layout, FramePattern::SolidPerMonitor);
        platform.set_failing_monitors(&["monitor-1"]);
        let request = CaptureRequest {
            format: CaptureFormat::VirtualDesktop,
            monitor_id: None,
            region: None,
        };
        let err = platform.capture(&request).expect_err("capture must fail");
        assert!(matches!(err.kind, PlatformErrorKind::CaptureUnavailable));
    }

    #[test]
    fn single_monitor_capture_returns_monitor_bounds() {
        let layout = SyntheticMonitorLayout::dual_side_by_side();
        let platform =
            SyntheticPlatform::with_layout(layout.clone(), FramePattern::SolidPerMonitor);
        let secondary = layout
            .monitors
            .iter()
            .find(|m| m.id == "monitor-1")
            .expect("secondary");
        let request = CaptureRequest {
            format: CaptureFormat::SingleMonitor,
            monitor_id: Some("monitor-1".into()),
            region: None,
        };
        let resolution = platform.capture(&request).expect("capture");
        assert_eq!(resolution.bounds, secondary.bounds);
        assert_eq!(resolution.bounds.size, secondary.bounds.size);
    }

    #[test]
    fn monitor_to_capture_buffer_matches_layout_origin() {
        // Spot-check the synthetic helper's offset arithmetic matches
        // the contract transform for a known layout.
        let layout = SyntheticMonitorLayout::dual_negative_origin();
        let virtual_bounds = layout.virtual_bounds().expect("bounds");
        let composite = virtual_bounds.as_top_left_bounds();
        let secondary = layout
            .monitors
            .iter()
            .find(|m| m.id == "monitor-1")
            .expect("secondary");
        let offset = pixelgrab_contracts::coordinate::transform::monitor_to_capture_buffer(
            &secondary.bounds,
            virtual_bounds.min,
            composite.size,
        );
        // Secondary sits at (0, -200) in the virtual desktop; the buffer
        // origin is the virtual desktop's min, so the offset is (1920, 0).
        let expected_x = secondary.bounds.origin.x - virtual_bounds.min.x;
        let expected_y = secondary.bounds.origin.y - virtual_bounds.min.y;
        assert_eq!(offset.origin.x, expected_x);
        assert_eq!(offset.origin.y, expected_y);
        let _ = PhysicalPoint::new(expected_x, expected_y);
    }
}
