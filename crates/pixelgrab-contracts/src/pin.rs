//! Pin-platform contracts.
//!
//! A pin is an always-on-top reference window that displays a captured image
//! independent of the shelf. The pin survives the user opening other pins,
//! moving them across monitors, zooming, and changing opacity. The contracts
//! in this crate are platform-neutral: pinning logic builds on top of them
//! and the platform-specific implementation (Windows / synthetic) plugs in
//! behind the [`PinLockProvider`] trait.
//!
//! The terminology used here matches `docs/GLOSSARY.md`:
//!
//! - **Pin** — a TopMost reference window that displays a captured image.
//! - **Pin lock** — a guard that prevents the cache from pruning the capture
//!   while the pin is visible. Released exactly once on teardown.
//! - **Pin view model** — the per-pin transform (position, zoom, opacity) and
//!   source metadata that the UI binds to.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::coordinate::{PhysicalBounds, PhysicalPoint, PhysicalSize};

/// Inclusive-exclusive bounds. Zoom and opacity are clamped to these at
/// every mutation so the UI cannot push the view model into an invalid state.
pub mod limits {
    /// Minimum zoom (20%).
    pub const MIN_ZOOM: f32 = 0.20;
    /// Maximum zoom (400%).
    pub const MAX_ZOOM: f32 = 4.00;
    /// Minimum opacity (20%).
    pub const MIN_OPACITY: f32 = 0.20;
    /// Maximum opacity (100%).
    pub const MAX_OPACITY: f32 = 1.00;
    /// Default zoom (100%).
    pub const DEFAULT_ZOOM: f32 = 1.00;
    /// Default opacity (100%).
    pub const DEFAULT_OPACITY: f32 = 1.00;
}

/// Stable identifier for a pin. Assigned by the registry at open time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PinId(pub String);

impl PinId {
    /// Convenience constructor.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PinId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Lifecycle state of a pin. The view model moves through these as the user
/// interacts; the registry may close the pin via any of the documented
/// close routes (escape, double-click, visible close control, reset, etc.)
/// and the state always converges to `Closed` on teardown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinLifecycle {
    /// Window is visible on the desktop.
    Open,
    /// Native window destroyed and the cache lock released.
    Closed,
}

/// Wire shape for a per-pin transform. The window size is the source size
/// scaled by `zoom`; the position is the physical-pixel top-left of the
/// window's client area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinTransform {
    /// Physical-pixel top-left of the window.
    pub position: PhysicalPoint,
    /// Physical-pixel size of the window (after zoom).
    pub window_size: PhysicalSize,
    /// Source pixel size (the captured image before zoom).
    pub source_size: PhysicalSize,
    /// Current zoom factor (1.0 = 100%).
    pub zoom: f32,
    /// Current opacity (1.0 = fully opaque).
    pub opacity: f32,
}

impl PinTransform {
    /// Construct a transform at the given position with the default zoom and
    /// opacity.
    pub fn at(position: PhysicalPoint, source_size: PhysicalSize) -> Self {
        Self {
            position,
            window_size: source_size,
            source_size,
            zoom: limits::DEFAULT_ZOOM,
            opacity: limits::DEFAULT_OPACITY,
        }
    }

    /// Re-derive the window size from the source size and zoom.
    pub fn with_zoom(mut self, zoom: f32) -> Self {
        self.zoom = clamp_zoom(zoom);
        self.window_size = scaled(self.source_size, self.zoom);
        self
    }

    /// Set the opacity (clamped to its bounds).
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = clamp_opacity(opacity);
        self
    }
}

/// Wire shape for a pin's view of its source. The UI uses this to render the
/// pin and to issue copy / save-as actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinSource {
    /// Capture id from the original capture pipeline.
    pub capture_id: String,
    /// Absolute path to the PNG on disk. None only when the pin is being
    /// closed and the source has been reaped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png_path: Option<String>,
    /// Physical-pixel bounds of the captured region (matches the source
    /// framebuffer).
    pub bounds: PhysicalBounds,
}

/// Wire shape for a pin's observable state. The UI binds to this every
/// animation frame; the Rust side recomputes it on every command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinViewModel {
    /// Pin id.
    pub id: PinId,
    /// Per-pin transform.
    pub transform: PinTransform,
    /// Source image metadata.
    pub source: PinSource,
}

/// Open a pin. The shell ships the metadata; the registry owns the lock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPinRequest {
    /// Capture id the pin will display.
    pub capture_id: String,
    /// Path to the PNG on disk.
    pub png_path: String,
    /// Physical-pixel bounds of the captured region.
    pub bounds: PhysicalBounds,
    /// Optional initial position. None places the pin at the origin of the
    /// primary monitor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_position: Option<PhysicalPoint>,
}

/// Command applied to a pin. The view model is pure: every command produces
/// a new transform; the registry is the only place that mutates state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PinCommand {
    /// Drag the pin by a physical-pixel delta. The origin moves; the size
    /// is unchanged.
    Drag {
        /// Physical-pixel delta.
        dx: i32,
        /// Physical-pixel delta.
        dy: i32,
    },
    /// Apply a multiplicative zoom factor at the cursor position. The cursor
    /// position is in the window's client coordinates (origin = top-left).
    Zoom {
        /// Multiplicative zoom factor (e.g. 1.10 for +10%).
        factor: f32,
        /// Cursor x in the window's client coordinates (CSS pixels).
        cursor_x: f32,
        /// Cursor y in the window's client coordinates (CSS pixels).
        cursor_y: f32,
    },
    /// Adjust opacity by a multiplicative factor (Ctrl+wheel).
    SetOpacity {
        /// Absolute opacity to apply (clamped to its bounds).
        opacity: f32,
    },
    /// Reset zoom and opacity to their defaults without moving the window.
    Reset,
}

/// Action requested via a pin's context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinAction {
    /// Copy the source PNG to the clipboard.
    Copy,
    /// Open the native Save As dialog and write the source PNG.
    SaveAs,
    /// Reset zoom and opacity to 100%.
    Reset,
    /// Close the pin.
    Close,
}

/// Result of a pin action. The `Copy` and `SaveAs` actions report the
/// outcome back to the UI so it can show a confirmation toast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinActionOutcome {
    /// Pin id the action was applied to.
    pub pin_id: PinId,
    /// Action that was performed.
    pub action: PinAction,
    /// Optional payload. For `Copy` and `SaveAs` this is the byte length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Optional PNG path written for `SaveAs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png_path: Option<String>,
}

/// Trait implemented by the cache layer. The pin registry acquires a lock
/// when a pin opens and releases it when the pin closes. The default
/// implementation is a no-op so the synthetic and test paths can use it
/// without a real cache.
pub trait PinLockProvider: std::fmt::Debug + Send + Sync {
    /// Acquire a lock on `capture_id`. Returns `true` if the lock was newly
    /// acquired, `false` if a previous lock was already held by the same
    /// provider. The implementation must be idempotent so a double-open
    /// path does not strand the lock.
    fn acquire(&self, capture_id: &str) -> bool;

    /// Release a lock on `capture_id`. Returns `true` if the lock was
    /// released, `false` if no lock was held (the registry tests assert
    /// the lock is released exactly once, so a "false" outcome is treated
    /// as a bug).
    fn release(&self, capture_id: &str) -> bool;

    /// Active lock count for diagnostics. The pin registry asserts this
    /// hits zero across repeated open/close cycles.
    fn active_locks(&self) -> usize;
}

/// Clamp a zoom factor into the inclusive bounds.
pub fn clamp_zoom(zoom: f32) -> f32 {
    if !zoom.is_finite() {
        return limits::DEFAULT_ZOOM;
    }
    zoom.clamp(limits::MIN_ZOOM, limits::MAX_ZOOM)
}

/// Clamp an opacity into the inclusive bounds.
pub fn clamp_opacity(opacity: f32) -> f32 {
    if !opacity.is_finite() {
        return limits::DEFAULT_OPACITY;
    }
    opacity.clamp(limits::MIN_OPACITY, limits::MAX_OPACITY)
}

/// Compute the new window size for a given source size and zoom.
pub fn scaled(source: PhysicalSize, zoom: f32) -> PhysicalSize {
    let zoom = clamp_zoom(zoom);
    let width = ((source.width as f32) * zoom).round().max(1.0) as u32;
    let height = ((source.height as f32) * zoom).round().max(1.0) as u32;
    PhysicalSize::new(width, height)
}

/// Apply a cursor-centered zoom. The math:
///
/// 1. The cursor is at pixel `c` on the screen (relative to the window).
/// 2. After zoom, the image under the cursor must still be the same pixel.
/// 3. The window's top-left therefore moves from `p` to `p' = p + c*(1 - z)`
///    where `z` is the new zoom factor and `c` is the cursor in window
///    coordinates. (`z > 1` moves the window left/up so the cursor stays
///    over the same image pixel.)
///
/// This keeps the world coordinate under the cursor invariant across zoom
/// changes.
pub fn cursor_centered_zoom(
    position: PhysicalPoint,
    cursor: PhysicalPoint,
    factor: f32,
    zoom: f32,
) -> (PhysicalPoint, f32) {
    let new_zoom = clamp_zoom(zoom * factor);
    let delta_x = (cursor.x as f32) * (1.0 - new_zoom / zoom.clamp(f32::MIN_POSITIVE, f32::MAX));
    let delta_y = (cursor.y as f32) * (1.0 - new_zoom / zoom.clamp(f32::MIN_POSITIVE, f32::MAX));
    let new_x = clamp_position_axis(position.x as f32 + delta_x);
    let new_y = clamp_position_axis(position.y as f32 + delta_y);
    (PhysicalPoint::new(new_x, new_y), new_zoom)
}

fn clamp_position_axis(value: f32) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let clamped = value.clamp(-(i32::MAX as f32), i32::MAX as f32);
    clamped.round() as i32
}

/// Re-anchor a pin so its top-left falls inside the given work area. If the
/// pin already intersects the work area, the position is unchanged; if it
/// is fully outside, the top-left is moved just inside the work area.
pub fn reanchor(
    position: PhysicalPoint,
    window_size: PhysicalSize,
    work_area: PhysicalBounds,
) -> PhysicalPoint {
    let window_bounds = PhysicalBounds::new(position, window_size);
    if work_area.intersect(&window_bounds) != PhysicalBounds::EMPTY {
        return position;
    }
    // Snap the top-left just inside the work area. We don't attempt to
    // preserve the previous cursor-relative offset because the geometry
    // is undefined once the pin is fully outside any work area.
    let x = anchored_axis(
        position.x,
        window_size.width,
        work_area.origin.x,
        work_area.right(),
    );
    let y = anchored_axis(
        position.y,
        window_size.height,
        work_area.origin.y,
        work_area.bottom(),
    );
    PhysicalPoint::new(x, y)
}

/// Snap one axis of a position so the window lands inside the work area.
/// If the window is wider than the work area, the only choice is the
/// work-area origin. Otherwise, snap left if the window is past the left
/// edge, or right if it is past the right edge.
fn anchored_axis(position: i32, window_extent: u32, work_origin: i32, work_end: i32) -> i32 {
    let work_extent = (work_end - work_origin).max(0) as u32;
    // Window is too wide for the work area, OR the window has fully
    // escaped past the left edge of the work area: in both cases the
    // only safe origin is the work-area origin.
    if window_extent >= work_extent || position + (window_extent as i32) < work_origin {
        work_origin
    } else if position > work_end {
        work_end - (window_extent as i32)
    } else {
        position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> PhysicalSize {
        PhysicalSize::new(200, 100)
    }

    #[test]
    fn clamp_zoom_handles_non_finite() {
        // Non-finite values collapse to the default (matching the
        // `round_to_i32` convention in `coordinate.rs`): a corrupt input
        // becomes a safe value rather than a wildly out-of-range one.
        assert_eq!(clamp_zoom(f32::NAN), limits::DEFAULT_ZOOM);
        assert_eq!(clamp_zoom(f32::INFINITY), limits::DEFAULT_ZOOM);
        assert_eq!(clamp_zoom(f32::NEG_INFINITY), limits::DEFAULT_ZOOM);
        // Out-of-range finite values clamp to the limit.
        assert_eq!(clamp_zoom(-1.0), limits::MIN_ZOOM);
        assert_eq!(clamp_zoom(99.0), limits::MAX_ZOOM);
        assert_eq!(clamp_zoom(0.5), 0.5);
    }

    #[test]
    fn clamp_opacity_handles_non_finite() {
        assert_eq!(clamp_opacity(f32::NAN), limits::DEFAULT_OPACITY);
        assert_eq!(clamp_opacity(2.0), limits::MAX_OPACITY);
        assert_eq!(clamp_opacity(0.0), limits::MIN_OPACITY);
    }

    #[test]
    fn scaled_applies_zoom() {
        let zoomed = scaled(source(), 2.0);
        assert_eq!(zoomed, PhysicalSize::new(400, 200));
        let half = scaled(source(), 0.5);
        assert_eq!(half, PhysicalSize::new(100, 50));
    }

    #[test]
    fn scaled_clamps_zero_to_one() {
        let tiny = scaled(PhysicalSize::new(0, 0), 0.5);
        assert_eq!(tiny, PhysicalSize::new(1, 1));
    }

    #[test]
    fn cursor_centered_zoom_keeps_pixel_under_cursor() {
        // Window at (100, 100), size 200x100. Cursor at (50, 50) in window
        // coords. Zoom in by 2x. The pixel under the cursor must stay
        // under the cursor (so the window's top-left moves toward the
        // cursor).
        let (new_pos, new_zoom) = cursor_centered_zoom(
            PhysicalPoint::new(100, 100),
            PhysicalPoint::new(50, 25),
            2.0,
            1.0,
        );
        assert!((new_zoom - 2.0).abs() < 1e-3, "zoom should double");
        // The cursor is at world-position (100+50, 100+25) before and after.
        // After zoom the window is larger so the top-left moves to keep
        // the cursor over the same image pixel.
        assert!(
            new_pos.x < 100,
            "zoom-in moves window left (got {})",
            new_pos.x
        );
        assert!(
            new_pos.y < 100,
            "zoom-in moves window up (got {})",
            new_pos.y
        );
    }

    #[test]
    fn cursor_centered_zoom_clamps_to_bounds() {
        let (_pos, zoom) = cursor_centered_zoom(
            PhysicalPoint::new(0, 0),
            PhysicalPoint::new(10, 10),
            100.0,
            1.0,
        );
        assert_eq!(zoom, limits::MAX_ZOOM);
    }

    #[test]
    fn reanchor_noop_when_inside() {
        let pos = PhysicalPoint::new(50, 50);
        let work = PhysicalBounds::from_xywh(0, 0, 1000, 1000);
        assert_eq!(reanchor(pos, PhysicalSize::new(100, 100), work), pos);
    }

    #[test]
    fn reanchor_moves_when_offscreen_left() {
        let pos = PhysicalPoint::new(-500, 50);
        let work = PhysicalBounds::from_xywh(0, 0, 1000, 1000);
        let new_pos = reanchor(pos, PhysicalSize::new(100, 100), work);
        assert!(new_pos.x >= work.origin.x);
        assert!(new_pos.x + 100 <= work.right());
    }

    #[test]
    fn reanchor_moves_when_offscreen_right() {
        let pos = PhysicalPoint::new(2000, 50);
        let work = PhysicalBounds::from_xywh(0, 0, 1000, 1000);
        let new_pos = reanchor(pos, PhysicalSize::new(100, 100), work);
        assert!(new_pos.x + 100 <= work.right());
        assert!(new_pos.x >= work.origin.x);
    }
}
