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
            .finish()
    }
}

struct EngineState {
    /// Last successful frozen frame, or None if no capture has run yet.
    frozen: Option<FrozenFrame>,
    /// Cached monitor layout from the most recent `monitor_layout()` call.
    layout: Option<MonitorLayout>,
}

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
            })),
        }
    }

    /// Return the cached monitor layout, querying the OS if not yet cached.
    pub fn monitor_layout(&self) -> PlatformResult<MonitorLayout> {
        let mut state = self.inner.lock();
        if let Some(layout) = &state.layout {
            return Ok(layout.clone());
        }
        let layout = query_monitor_layout().map_err(map_xcap_err)?;
        state.layout = Some(layout.clone());
        Ok(layout)
    }

    /// Run a capture pipeline. The captured framebuffer is stored in the
    /// engine and the resulting `CaptureResolution` references it by id.
    pub fn capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution> {
        let bounds = match request.format {
            CaptureFormat::VirtualDesktop => self.virtual_desktop_bounds()?,
            CaptureFormat::SingleMonitor => {
                let id = request.monitor_id.as_deref().ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorKind::InvalidPayload,
                        "SingleMonitor format requires monitor_id",
                    )
                })?;
                let layout = self.monitor_layout()?;
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
        let rgba = capture_monitor(&bounds).map_err(map_xcap_err)?;
        let captured_at_ms = now_ms();
        let frame = FrozenFrame {
            capture_id: capture_id.clone(),
            bounds,
            rgba: Arc::new(rgba),
            captured_at_ms,
        };
        let asset_url = encode_png_data_url(&frame)?;
        let resolution = CaptureResolution {
            format: request.format,
            bounds,
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

    fn virtual_desktop_bounds(&self) -> PlatformResult<PhysicalBounds> {
        let layout = self.monitor_layout()?;
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for monitor in &layout.monitors {
            min_x = min_x.min(monitor.bounds.origin.x);
            min_y = min_y.min(monitor.bounds.origin.y);
            max_x = max_x.max(monitor.bounds.right());
            max_y = max_y.max(monitor.bounds.bottom());
        }
        if min_x == i32::MAX {
            return Err(CaptureError::MonitorEnumeration("no monitors detected".into()).into());
        }
        Ok(PhysicalBounds::from_xywh(
            min_x,
            min_y,
            (max_x - min_x) as u32,
            (max_y - min_y) as u32,
        ))
    }
}

impl Default for CaptureEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Query the current monitor layout from Windows via `xcap`.
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
        // `xcap` does not currently report work-area insets; use the full
        // bounds until a richer source becomes available.
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
    Ok(MonitorLayout::new(descriptors))
}

/// Capture the pixels for the given physical bounds. Routes through the
/// Capture the pixels for the given physical bounds. Routes through the
/// primary monitor when one exists, falling back to the first enumerated
/// monitor when no primary is reported. The chosen monitor's local
/// coordinate space is used to clip the requested bounds so a partially
/// off-screen request never causes the capture pipeline to fail.
///
/// Multi-monitor stitching (requesting bounds that span more than one
/// monitor) is intentionally out of scope for tracer-02; the tracer-04
/// issue owns that capability.
fn capture_monitor(bounds: &PhysicalBounds) -> Result<Vec<u8>, xcap::XCapError> {
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
        CaptureError::Pipeline(_) => "capture_unavailable",
        CaptureError::InvalidOutput(_) => "capture_unavailable",
        CaptureError::CropOutOfBounds(_) => "coordinate_transform",
        CaptureError::CoordinateTransform(_) => "coordinate_transform",
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
