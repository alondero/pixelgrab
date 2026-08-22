//! Native Windows capture engine. Wraps the `xcap` crate so the rest of the
//! PixelGrab codebase never depends on a specific capture library.
//!
//! `CaptureEngine` is the only path through which pixels enter the
//! orchestrator. The capture always runs against the *primary* monitor
//! (or the virtual desktop for `CaptureFormat::VirtualDesktop`) before
//! the overlay window is revealed, so the overlay is never present in its
//! own captured frame. The frozen framebuffer is retained for the lifetime
//! of the session so the commit pipeline can crop without re-capturing.

use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use pixelgrab_contracts::{
    capture::{CaptureFormat, CaptureRequest, CaptureResolution},
    coordinate::{PhysicalBounds, PhysicalSize},
    monitor::{MonitorDescriptor, MonitorLayout},
    CaptureDiagnostics, PlatformError, PlatformErrorKind, PlatformResult,
};

use super::super::contract::CaptureError;

/// Owns the in-memory frozen framebuffer and the underlying `xcap` monitor
/// layout. Cloning is cheap; the captured bytes are reference-counted.
#[derive(Clone)]
pub struct CaptureEngine {
    inner: Arc<Mutex<EngineState>>,
}

impl fmt::Debug for CaptureEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.lock();
        f.debug_struct("CaptureEngine")
            .field("has_frozen", &state.frozen.is_some())
            .field("layout_cached", &state.layout.is_some())
            .field("topology_changed", &state.topology_dirty)
            .finish()
    }
}

struct EngineState {
    /// Last successful frozen frame, or None if no capture has run yet.
    frozen: Option<FrozenFrame>,
    /// Cached monitor layout from the most recent `monitor_layout()` call.
    layout: Option<MonitorLayout>,
    /// `true` when the cached layout is suspect and must be re-queried
    /// before the next capture. Set by `invalidate_layout` and by the
    /// hot-plug/unplug detection loop. The capture pipeline treats this as
    /// authoritative — a stale layout could place pixels against the wrong
    /// monitor offsets.
    topology_dirty: bool,
    /// Issue #63: when set, capture frames are persisted under this root
    /// (bounded local asset transport) and the resolution carries the
    /// file path instead of an inline base64 data URL.
    frame_cache_root: Option<std::path::PathBuf>,
}

/// Maximum number of physical pixels the composite framebuffer is allowed
/// to allocate. The cap is intentionally generous (32K x 32K pixels,
/// 32 GiB at 4 bytes per pixel) so the cap only fires on a malformed
/// layout. The cap protects the process from a runaway enumeration
/// (e.g. a virtual display driver reporting absurd sizes) and satisfies
/// the tracer-03 acceptance criterion "Bound framebuffer allocation".
pub const MAX_FRAMEBUFFER_PIXELS: u64 = 32 * 1024 * 1024 * 1024 / 4; // 32 GiB worth of pixels.

/// Comfort cap on how many monitors a single composite pipeline accepts.
/// `xcap` reports physical monitors; any number larger than this is a
/// virtual display driver or a stuck enumeration. Coil keeps the
/// composite bounded so a single capture cannot fan out to dozens of
/// captures that block the orchestrator.
pub const MAX_MONITORS_PER_CAPTURE: usize = 32;

/// An immutable RGBA frame captured from the Windows desktop.
#[derive(Debug, Clone)]
pub struct FrozenFrame {
    /// Capture id assigned by the caller (so log lines correlate with the
    /// `CaptureResolution`).
    pub capture_id: String,
    /// Physical bounds of the captured framebuffer (origin in the virtual
    /// desktop coordinate system).
    pub bounds: PhysicalBounds,
    /// RGBA pixel buffer, tightly packed, `width * height * 4` bytes.
    pub rgba: Arc<Vec<u8>>,
    /// Wall-clock milliseconds when the capture completed.
    pub captured_at_ms: i64,
}

impl FrozenFrame {
    /// Crop the frozen framebuffer to the requested physical bounds.
    /// Returns an error if the crop escapes the framebuffer's extent.
    pub fn crop(&self, crop: &PhysicalBounds) -> PlatformResult<Vec<u8>> {
        let buffer_origin = self.bounds.origin;
        let buffer_size = self.bounds.size;
        if crop.size.width == 0 || crop.size.height == 0 {
            return Err(
                CaptureError::CropOutOfBounds("crop has zero width or height".into()).into(),
            );
        }
        let crop_in_buffer = pixelgrab_contracts::coordinate::transform::physical_to_capture_buffer(
            crop,
            buffer_origin,
        );
        let clamped = pixelgrab_contracts::coordinate::transform::clamp_to_capture_buffer(
            &crop_in_buffer,
            buffer_size,
        );
        if clamped.size.width == 0 || clamped.size.height == 0 {
            return Err(CaptureError::CropOutOfBounds(
                "crop lies outside the captured framebuffer".into(),
            )
            .into());
        }
        copy_region(&self.rgba, buffer_size, clamped)
    }
}

/// Return the current wall-clock time in milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl CaptureEngine {
    /// Construct a fresh engine. No captures have been performed yet.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EngineState {
                frozen: None,
                layout: None,
                topology_dirty: true,
                frame_cache_root: None,
            })),
        }
    }

    /// Configure the root under which capture frames are persisted for
    /// the bounded local asset transport (issue #63). When unset the
    /// engine falls back to inline data URLs (synthetic / CI paths).
    pub fn set_frame_cache_root(&self, root: Option<std::path::PathBuf>) {
        self.inner.lock().frame_cache_root = root;
    }

    /// Return the cached monitor layout, querying the OS if not yet cached.
    /// The cache is invalidated by [`CaptureEngine::invalidate_layout`] so
    /// hot-plug/unplug events force a fresh enumeration before the next
    /// capture.
    pub fn monitor_layout(&self) -> PlatformResult<MonitorLayout> {
        let mut state = self.inner.lock();
        let dirty = state.topology_dirty;
        if !dirty {
            if let Some(layout) = &state.layout {
                return Ok(layout.clone());
            }
        }
        let layout = query_monitor_layout().map_err(map_xcap_err)?;
        state.layout = Some(layout.clone());
        state.topology_dirty = false;
        Ok(layout)
    }

    /// Mark the cached layout as suspect. The next call to
    /// `monitor_layout()` re-queries the OS. The platform contract calls
    /// this on `WM_DISPLAYCHANGE` (or its polled equivalent) so a
    /// re-enumeration happens before the next capture, not after.
    pub fn invalidate_layout(&self) {
        let mut state = self.inner.lock();
        state.topology_dirty = true;
        // The frozen frame was captured against the previous topology; it
        // is no longer trustworthy. The session must observe this on
        // commit (the `flatten_crop` path rejects the stale id) so we
        // also drop the cached frame. The orchestrator will be notified
        // through the next `monitor_layout()` call instead.
        state.frozen = None;
    }

    /// Returns `true` when the cached layout has been invalidated since
    /// the last query. Used by the orchestrator to debug timeline
    /// regressions; the capture pipeline itself ignores the flag.
    pub fn is_topology_dirty(&self) -> bool {
        self.inner.lock().topology_dirty
    }

    /// Run a capture pipeline. The captured framebuffer is stored in the
    /// engine and the resulting `CaptureResolution` references it by id.
    ///
    /// `VirtualDesktop` captures fan out to every active monitor in
    /// parallel (`xcap` runs on a background thread per monitor) and
    /// composite the per-monitor framebuffers into one RGBA virtual
    /// framebuffer. `SingleMonitor` captures one monitor at its native
    /// resolution. `PhysicalRegion` captures the requested region against
    /// whichever monitor it overlaps; multi-monitor spans are not yet
    /// supported through this path (the issue is owned by a future
    /// tracer) — callers wanting a stitched result go through
    /// `VirtualDesktop` instead.
    pub fn capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution> {
        let layout = self.monitor_layout()?;
        let bounds = match request.format {
            CaptureFormat::VirtualDesktop => virtual_bounds_from_layout(&layout)?,
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

        if bounds.size.width == 0 || bounds.size.height == 0 {
            return Err(CaptureError::InvalidOutput(
                "capture bounds have zero width or height".into(),
            )
            .into());
        }

        let capture_id = uuid::Uuid::new_v4().to_string();
        let captured_at_ms = now_ms();
        let frame = match request.format {
            CaptureFormat::VirtualDesktop => {
                // Per-monitor fan-out: the composite pipeline captures every
                // monitor in parallel and blits the per-monitor frames
                // into one RGBA virtual framebuffer.
                let cached_layout = layout.clone();
                let (rgba, composite_bounds) = composite_virtual_desktop(&cached_layout)?;
                FrozenFrame {
                    capture_id: capture_id.clone(),
                    bounds: composite_bounds,
                    rgba: Arc::new(rgba),
                    captured_at_ms,
                }
            }
            CaptureFormat::SingleMonitor | CaptureFormat::PhysicalRegion => {
                let rgba = capture_single_monitor(&bounds).map_err(map_xcap_err)?;
                FrozenFrame {
                    capture_id: capture_id.clone(),
                    bounds,
                    rgba: Arc::new(rgba),
                    captured_at_ms,
                }
            }
        };
        // Issue #63: bounded local asset transport — encode once, write
        // the PNG under the frame cache root, and hand the webview a
        // file path (loaded via the asset protocol) instead of a
        // multi-megabyte base64 string crossing IPC.
        let png_bytes = encode_png(&frame.rgba, frame.bounds.size)?;
        let asset_url = {
            let state = self.inner.lock();
            crate::platform::asset::write_capture_asset(
                state.frame_cache_root.as_deref(),
                &capture_id,
                &png_bytes,
            )?
        };
        let resolution = CaptureResolution {
            format: request.format,
            bounds: frame.bounds,
            asset_url,
            capture_id,
            captured_at_ms,
        };
        self.inner.lock().frozen = Some(frame);
        Ok(resolution)
    }

    /// Take the frozen frame (and clear the engine state) so the session
    /// orchestrator owns the bytes for the commit pipeline. Errors if no
    /// frame is currently frozen.
    pub fn take_frozen(&self) -> PlatformResult<FrozenFrame> {
        self.inner.lock().frozen.take().ok_or_else(|| {
            CaptureError::Pipeline("no frozen frame is available to commit".into()).into()
        })
    }

    /// Borrow the frozen frame without consuming it. Used by tests and
    /// diagnostics.
    pub fn frozen(&self) -> Option<FrozenFrame> {
        self.inner.lock().frozen.clone()
    }

    /// Drop the frozen frame, returning the engine to its empty state.
    pub fn clear(&self) {
        self.inner.lock().frozen = None;
    }
}

/// Derive the virtual desktop bounds from a `MonitorLayout`. Returns the
/// inclusive-min / exclusive-max rectangle the composite framebuffer should
/// cover. Pure function — also used by the synthetic adapter so the two
/// platforms stay aligned on the rounding policy.
fn virtual_bounds_from_layout(layout: &MonitorLayout) -> PlatformResult<PhysicalBounds> {
    let vb = layout
        .virtual_bounds()
        .ok_or_else(|| CaptureError::MonitorEnumeration("no monitors detected".into()))?;
    Ok(vb.as_top_left_bounds())
}

impl Default for CaptureEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Query the current monitor layout from Windows via `xcap`, then
/// merge the real per-monitor work areas reported by `GetMonitorInfoW`
/// (issue #63 — `xcap` does not report work-area insets, so the shelf
/// placement math would otherwise treat the taskbar as usable space).
fn query_monitor_layout() -> Result<MonitorLayout, xcap::XCapError> {
    let monitors = xcap::Monitor::all()?;
    let mut descriptors = Vec::with_capacity(monitors.len());
    for (idx, monitor) in monitors.iter().enumerate() {
        let id = monitor
            .id()
            .map(|value| value.to_string())
            .unwrap_or_else(|_| format!("monitor-{idx}"));
        let label = monitor.name().unwrap_or_else(|_| id.clone());
        let is_primary = monitor.is_primary().unwrap_or(false);
        let x = monitor.x().unwrap_or(0);
        let y = monitor.y().unwrap_or(0);
        let width = monitor.width().unwrap_or(0);
        let height = monitor.height().unwrap_or(0);
        let scale_factor = monitor.scale_factor().unwrap_or(1.0);
        let bounds = PhysicalBounds::from_xywh(x, y, width, height);
        // Placeholder until the `apply_work_areas` merge below replaces
        // it with the real `rcWork` rect.
        let work_area = bounds;
        descriptors.push(MonitorDescriptor {
            id,
            label,
            is_primary,
            bounds,
            scale_factor,
            work_area,
        });
    }
    if descriptors.is_empty() {
        return Err(xcap::XCapError::new("no monitors returned by xcap"));
    }
    let mut layout = MonitorLayout::new(descriptors);
    let raw_work_areas = super::work_area::ffi::query_raw_work_areas();
    if !raw_work_areas.is_empty() {
        layout = super::work_area::apply_work_areas(&layout, &raw_work_areas);
    }
    Ok(layout)
}

/// Capture the pixels for the given physical bounds. Routes through the
/// primary monitor when one exists, falling back to the first enumerated
/// monitor when no primary is reported. The chosen monitor's local
/// coordinate space is used to clip the requested bounds so a partially
/// off-screen request never causes the capture pipeline to fail.
fn capture_single_monitor(bounds: &PhysicalBounds) -> Result<Vec<u8>, xcap::XCapError> {
    let monitors = xcap::Monitor::all()?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .ok_or_else(|| xcap::XCapError::new("no monitor available for capture"))?;
    let monitor_bounds = physical_bounds_of(monitor);
    let local_x = (bounds.origin.x - monitor_bounds.origin.x).max(0) as u32;
    let local_y = (bounds.origin.y - monitor_bounds.origin.y).max(0) as u32;
    let local_w = bounds
        .size
        .width
        .min(monitor_bounds.size.width.saturating_sub(local_x));
    let local_h = bounds
        .size
        .height
        .min(monitor_bounds.size.height.saturating_sub(local_y));
    if local_w == 0 || local_h == 0 {
        return Err(xcap::XCapError::new(
            "requested capture lies entirely outside the chosen monitor",
        ));
    }
    let image = monitor.capture_region(local_x, local_y, local_w, local_h)?;
    Ok(image.into_raw())
}

/// Result of a single monitor's parallel capture. The pixels are
/// tightly-packed RGBA bytes with the monitor's full size; the compositor
/// blits them into the virtual framebuffer at the offset reported by
/// [`MonitorCapture::buffer_offset`].
struct MonitorCapture {
    /// The monitor id (from the `MonitorLayout`).
    #[allow(dead_code)]
    monitor_id: String,
    /// `PhysicalBounds` in the captured framebuffer's local coordinate
    /// space where the captured pixels should be blitted.
    buffer_offset: PhysicalBounds,
    /// RGBA pixels, tightly packed, `width * height * 4` bytes.
    rgba: Vec<u8>,
}

/// Composite pipeline: fan out one `xcap::Monitor::capture_image` per
/// monitor on a worker thread, then blit each monitor's framebuffer into
/// the virtual framebuffer at the offset that corresponds to the
/// virtual-desktop origin. The composite is rejected if any monitor
/// fails — the platform never commits a partial desktop as a complete
/// capture.
fn composite_virtual_desktop(layout: &MonitorLayout) -> PlatformResult<(Vec<u8>, PhysicalBounds)> {
    if layout.monitors.is_empty() {
        return Err(CaptureError::MonitorEnumeration("no monitors detected".into()).into());
    }
    if layout.monitors.len() > MAX_MONITORS_PER_CAPTURE {
        return Err(CaptureError::FramebufferTooLarge {
            width: layout.monitors.len() as u32,
            height: 1,
        }
        .into());
    }

    let virtual_bounds = layout
        .virtual_bounds()
        .ok_or_else(|| CaptureError::MonitorEnumeration("no monitors detected".into()))?;
    let composite_bounds = virtual_bounds.as_top_left_bounds();
    let buffer_size = composite_bounds.size;
    let total_pixels = (buffer_size.width as u64) * (buffer_size.height as u64);
    if total_pixels > MAX_FRAMEBUFFER_PIXELS {
        return Err(CaptureError::FramebufferTooLarge {
            width: buffer_size.width,
            height: buffer_size.height,
        }
        .into());
    }

    // Fan out: one capture per monitor on a worker thread. The
    // `xcap::Monitor::capture_image` call is the long pole; parallel
    // capture on a typical 2-monitor desktop takes the same wall-clock
    // as the slowest monitor rather than the sum.
    let captures = capture_all_monitors_parallel(layout)?;
    let mut composite = vec![0u8; (buffer_size.width as usize) * (buffer_size.height as usize) * 4];
    for cap in &captures {
        blit_rgba(&mut composite, buffer_size, &cap.buffer_offset, &cap.rgba);
    }

    Ok((composite, composite_bounds))
}

/// Capture every monitor in parallel. Each monitor's framebuffer is
/// captured against the full monitor size so the compositor can blit it
/// without masking. The offset in the virtual framebuffer is computed
/// from the virtual desktop origin (the difference between the monitor's
/// physical origin and the virtual desktop's inclusive minimum).
///
/// `xcap::Monitor` is not `Send` (it wraps an `HMONITOR`), so we cannot
/// ship the monitor reference across threads. Each capture thread
/// re-queries the OS monitor list and locates the descriptor by id.
/// The cost is one extra `Monitor::all()` call per monitor, which is
/// negligible compared to the capture itself.
fn capture_all_monitors_parallel(layout: &MonitorLayout) -> PlatformResult<Vec<MonitorCapture>> {
    let virtual_bounds = layout
        .virtual_bounds()
        .ok_or_else(|| CaptureError::MonitorEnumeration("no monitors detected".into()))?;
    let buffer_size = virtual_bounds.as_top_left_bounds().size;

    // Spawn one thread per monitor. Each thread re-queries monitors and
    // captures the one matching its target id; the main thread joins
    // back into a single `MonitorCapture` list and blits.
    let handles: Vec<std::thread::JoinHandle<MonitorThreadResult>> = layout
        .monitors
        .iter()
        .map(|descriptor| {
            let id = descriptor.id.clone();
            let id_for_thread = id.clone();
            let handle = std::thread::Builder::new()
                .name(format!("pixelgrab-capture-{id_for_thread}"))
                .spawn(move || {
                    let image = capture_one_monitor(&id_for_thread);
                    MonitorThreadResult {
                        monitor_id: id_for_thread,
                        image,
                    }
                })
                .map_err(|err| {
                    CaptureError::Pipeline(format!("capture thread spawn failed for {id}: {err}"))
                })?;
            Ok(handle)
        })
        .collect::<PlatformResult<Vec<_>>>()?;

    let mut captures = Vec::with_capacity(handles.len());
    for handle in handles {
        let result = handle
            .join()
            .map_err(|_| CaptureError::Pipeline("capture worker thread panicked".into()))?;
        let xcap_image = match result.image {
            Ok(image) => image,
            Err(_err) => {
                // Privacy: keep the monitor id (categorical) but replace
                // the raw xcap error string with a stable kind. xcap
                // errors can echo COM HRESULTs that are not telemetry we
                // want to ship through the platform errors.
                log::warn!(
                    "monitor {} capture failed: xcap capture_image returned an error",
                    result.monitor_id
                );
                return Err(CaptureError::MonitorCaptureFailed {
                    monitor_id: result.monitor_id,
                    reason: "capture_image_failed".into(),
                }
                .into());
            }
        };
        let descriptor = layout
            .monitors
            .iter()
            .find(|m| m.id == result.monitor_id)
            .ok_or_else(|| {
                CaptureError::MonitorEnumeration(format!(
                    "monitor {} disappeared mid-capture",
                    result.monitor_id
                ))
            })?;
        let buffer_offset = pixelgrab_contracts::coordinate::transform::monitor_to_capture_buffer(
            &descriptor.bounds,
            virtual_bounds.min,
            buffer_size,
        );
        let raw = xcap_image.into_raw();
        if raw.len()
            != (descriptor.bounds.size.width as usize)
                * (descriptor.bounds.size.height as usize)
                * 4
        {
            return Err(CaptureError::InvalidOutput(format!(
                "monitor {} returned {} bytes; expected {}x{}x4",
                result.monitor_id,
                raw.len(),
                descriptor.bounds.size.width,
                descriptor.bounds.size.height
            ))
            .into());
        }
        captures.push(MonitorCapture {
            monitor_id: result.monitor_id,
            buffer_offset,
            rgba: raw,
        });
    }
    Ok(captures)
}

/// Capture a single monitor by id. Re-queries the OS monitor list inside
/// the current thread (xcap's `Monitor` is not `Send`) and matches by
/// the stable id assigned by `query_monitor_layout`. Returns the raw
/// RGBA buffer the caller can blit into a composite framebuffer.
fn capture_one_monitor(target_id: &str) -> Result<xcap::image::RgbaImage, xcap::XCapError> {
    let monitors = xcap::Monitor::all()?;
    let monitor = monitors
        .into_iter()
        .find(|m| matches!(m.id(), Ok(id) if id.to_string() == target_id))
        .ok_or_else(|| xcap::XCapError::new("monitor id not found during capture"))?;
    monitor.capture_image()
}

/// Internal result type for the per-monitor capture thread. The image
/// field is `Err` only when the xcap call itself failed; thread panics
/// are reported through the `JoinHandle::join` return.
struct MonitorThreadResult {
    monitor_id: String,
    image: Result<xcap::image::RgbaImage, xcap::XCapError>,
}

/// Copy an RGBA framebuffer into a destination framebuffer at the given
/// offset. The offset is in the destination's local coordinate space;
/// pixels that fall outside the destination are silently dropped.
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

fn physical_bounds_of(monitor: &xcap::Monitor) -> PhysicalBounds {
    PhysicalBounds::from_xywh(
        monitor.x().unwrap_or(0),
        monitor.y().unwrap_or(0),
        monitor.width().unwrap_or(0),
        monitor.height().unwrap_or(0),
    )
}

fn copy_region(
    src: &[u8],
    src_size: PhysicalSize,
    region: PhysicalBounds,
) -> PlatformResult<Vec<u8>> {
    let row_bytes = (region.size.width as usize) * 4;
    let mut out = vec![0u8; row_bytes * (region.size.height as usize)];
    let src_stride = (src_size.width as usize) * 4;
    for row in 0..(region.size.height as usize) {
        let src_offset =
            ((region.origin.y as usize) + row) * src_stride + (region.origin.x as usize) * 4;
        let dst_offset = row * row_bytes;
        out[dst_offset..dst_offset + row_bytes]
            .copy_from_slice(&src[src_offset..src_offset + row_bytes]);
    }
    Ok(out)
}

fn map_xcap_err(err: xcap::XCapError) -> PlatformError {
    use xcap::XCapError;
    // The set of variants that exist on Windows is much smaller than the
    // full union; the Windows-only enum drops the Linux / macOS variants.
    let kind = match &err {
        XCapError::InvalidCaptureRegion(_) => PlatformErrorKind::CoordinateTransform,
        XCapError::NotSupported => PlatformErrorKind::Unsupported,
        XCapError::Error(_) => PlatformErrorKind::CaptureUnavailable,
        XCapError::StdSyncPoisonError(_) => PlatformErrorKind::Internal,
        XCapError::WindowsCoreError(_) | XCapError::Utf16Error(_) => {
            PlatformErrorKind::CaptureUnavailable
        }
    };
    PlatformError::new(kind, format!("xcap capture failed: {err}"))
}

/// Encode the given RGBA buffer as a PNG and return a `data:` URL suitable
/// for direct loading by the WebView. Pure function used by both the
/// capture pipeline and the commit pipeline.
pub fn encode_png_data_url(frame: &FrozenFrame) -> PlatformResult<String> {
    let png_bytes = encode_png(&frame.rgba, frame.bounds.size)?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64_encode(&png_bytes)
    ))
}

/// Encode an RGBA buffer as a PNG. Exposed so the commit pipeline can
/// re-encode the flattened crop for the clipboard without duplicating the
/// encoder configuration.
pub fn encode_png(rgba: &[u8], size: PhysicalSize) -> PlatformResult<Vec<u8>> {
    let width = size.width;
    let height = size.height;
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
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
    let mut buf = Vec::with_capacity(expected + 1024);
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
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
            stream
                .write_all(rgba)
                .map_err(|e| PlatformError::new(PlatformErrorKind::Io, e.to_string()))?;
            stream
                .finish()
                .map_err(|e| PlatformError::new(PlatformErrorKind::Io, e.to_string()))?;
        }
    }
    Ok(buf)
}

/// Build the diagnostics record for a successful capture. Used by the
/// orchestrator to attribute latency without logging pixels.
pub fn diagnostics_for(
    capture_id: &str,
    monitor_id: &str,
    bounds: PhysicalBounds,
    started_at_ms: i64,
    completed_at_ms: i64,
) -> CaptureDiagnostics {
    CaptureDiagnostics::started(capture_id, monitor_id, bounds, started_at_ms)
        .completed(completed_at_ms)
}

/// Build a failure diagnostics record. Use only the categorical kind
/// string; never include the raw error message (it may echo user paths).
pub fn diagnostics_failure(
    capture_id: &str,
    monitor_id: &str,
    bounds: PhysicalBounds,
    started_at_ms: i64,
    failure: &CaptureError,
) -> CaptureDiagnostics {
    let kind = match failure {
        CaptureError::MonitorEnumeration(_) => "monitor_query_failed",
        CaptureError::UnsupportedFormat(_) => "unsupported",
        CaptureError::MonitorCaptureFailed { .. } => "monitor_capture_failed",
        CaptureError::Pipeline(_) => "capture_unavailable",
        CaptureError::InvalidOutput(_) => "capture_unavailable",
        CaptureError::CropOutOfBounds(_) => "coordinate_transform",
        CaptureError::CoordinateTransform(_) => "coordinate_transform",
        CaptureError::FramebufferTooLarge { .. } => "framebuffer_too_large",
    };
    CaptureDiagnostics::started(capture_id, monitor_id, bounds, started_at_ms)
        .completed(now_ms())
        .failed(kind)
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
    use pixelgrab_contracts::coordinate::transform;
    use pixelgrab_contracts::{ClientBounds, ClientPoint, ClientSize};

    fn filled_frame(w: u32, h: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                buf.push((x & 0xFF) as u8);
                buf.push((y & 0xFF) as u8);
                buf.push(0);
                buf.push(0xFF);
            }
        }
        buf
    }

    #[test]
    fn encode_png_rejects_bad_rgba_length() {
        let result = encode_png(&[0u8; 7], PhysicalSize::new(4, 4));
        assert!(result.is_err());
    }

    #[test]
    fn encode_png_round_trip_signature() {
        let frame = filled_frame(8, 8);
        let bytes = encode_png(&frame, PhysicalSize::new(8, 8)).expect("encode");
        assert!(bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));
    }

    #[test]
    fn frozen_frame_crop_extracts_region() {
        let rgba = Arc::new(filled_frame(8, 8));
        let frame = FrozenFrame {
            capture_id: "id".into(),
            bounds: PhysicalBounds::from_xywh(0, 0, 8, 8),
            rgba: rgba.clone(),
            captured_at_ms: 0,
        };
        let crop = PhysicalBounds::from_xywh(2, 2, 4, 4);
        let pixels = frame.crop(&crop).expect("crop");
        assert_eq!(pixels.len(), 4 * 4 * 4);
        // Top-left pixel of the crop should match (2, 2) of the source.
        assert_eq!(
            &pixels[0..4],
            &rgba[2 * 8 * 4 + 2 * 4..2 * 8 * 4 + 2 * 4 + 4]
        );
    }

    #[test]
    fn frozen_frame_crop_rejects_zero_size() {
        let rgba = Arc::new(filled_frame(8, 8));
        let frame = FrozenFrame {
            capture_id: "id".into(),
            bounds: PhysicalBounds::from_xywh(0, 0, 8, 8),
            rgba,
            captured_at_ms: 0,
        };
        let result = frame.crop(&PhysicalBounds::from_xywh(0, 0, 0, 0));
        assert!(result.is_err());
    }

    #[test]
    fn frozen_frame_crop_rejects_out_of_bounds() {
        let rgba = Arc::new(filled_frame(8, 8));
        let frame = FrozenFrame {
            capture_id: "id".into(),
            bounds: PhysicalBounds::from_xywh(0, 0, 8, 8),
            rgba,
            captured_at_ms: 0,
        };
        let crop = PhysicalBounds::from_xywh(8, 8, 4, 4);
        let result = frame.crop(&crop);
        assert!(result.is_err());
    }

    #[test]
    fn engine_starts_empty() {
        let engine = CaptureEngine::new();
        assert!(engine.frozen().is_none());
        assert!(engine.take_frozen().is_err());
    }

    #[test]
    fn diagnostics_record_captures_latency() {
        let bounds = PhysicalBounds::from_xywh(0, 0, 100, 100);
        let diag = diagnostics_for("id", "primary", bounds, 1_000, 1_025);
        assert_eq!(diag.capture_duration_ms, 25);
        assert!(diag.capture_to_overlay_ms.is_none());
        let diag = diag.overlay_visible(1_040);
        assert_eq!(diag.capture_to_overlay_ms, Some(15));
    }

    #[test]
    fn coordinate_transform_round_trip() {
        let capture_bounds = PhysicalBounds::from_xywh(0, 0, 1920, 1080);
        let stage = ClientSize::new(960.0, 540.0);
        let client =
            ClientBounds::new(ClientPoint::new(120.0, 60.0), ClientSize::new(480.0, 240.0));
        let physical = transform::client_to_physical(&client, capture_bounds, stage);
        assert_eq!(physical.origin.x, 240);
        assert_eq!(physical.origin.y, 120);
        assert_eq!(physical.size.width, 960);
        assert_eq!(physical.size.height, 480);
    }
}
