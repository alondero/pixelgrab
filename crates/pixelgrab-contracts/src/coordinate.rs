//! Physical-coordinate types. See ADR-0003 for the ownership rules.

use serde::{Deserialize, Serialize};

/// A 2D point in physical desktop pixels. Always non-negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalPoint {
    /// Pixels from the left edge.
    pub x: i32,
    /// Pixels from the top edge.
    pub y: i32,
}

impl PhysicalPoint {
    /// Convenience constructor.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A 2D size in physical desktop pixels. Both axes must be positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalSize {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl PhysicalSize {
    /// Convenience constructor.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// A physical-pixel rectangle expressed as origin + size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalBounds {
    /// Top-left corner.
    pub origin: PhysicalPoint,
    /// Extents.
    pub size: PhysicalSize,
}

impl PhysicalBounds {
    /// Empty bounds at the origin (zero width and height).
    pub const EMPTY: Self = Self {
        origin: PhysicalPoint::new(0, 0),
        size: PhysicalSize::new(0, 0),
    };

    /// Construct from origin and size.
    pub const fn new(origin: PhysicalPoint, size: PhysicalSize) -> Self {
        Self { origin, size }
    }

    /// Construct from raw coordinates.
    pub const fn from_xywh(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            origin: PhysicalPoint::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    /// Right edge (exclusive).
    pub fn right(&self) -> i32 {
        self.origin.x.saturating_add(self.size.width as i32)
    }

    /// Bottom edge (exclusive).
    pub fn bottom(&self) -> i32 {
        self.origin.y.saturating_add(self.size.height as i32)
    }

    /// `true` when both axes are zero (no selection).
    pub fn is_empty(&self) -> bool {
        self.size.width == 0 || self.size.height == 0
    }

    /// Validates that the bounds are non-degenerate and non-negative.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.size.width == 0 || self.size.height == 0 {
            return Err("bounds must have non-zero width and height");
        }
        if self.origin.x < 0 || self.origin.y < 0 {
            return Err("bounds origin must be non-negative");
        }
        Ok(())
    }

    /// Clamp to the given inclusive-exclusive extent.
    pub fn clamped_to(&self, extent: &PhysicalBounds) -> Self {
        let extent_right = extent.right();
        let extent_bottom = extent.bottom();
        let x = self.origin.x.max(extent.origin.x);
        let y = self.origin.y.max(extent.origin.y);
        let mut right = self.right().min(extent_right);
        let mut bottom = self.bottom().min(extent_bottom);
        if right < x {
            right = x;
        }
        if bottom < y {
            bottom = y;
        }
        let width = (right - x).max(0) as u32;
        let height = (bottom - y).max(0) as u32;
        Self::from_xywh(x, y, width, height)
    }

    /// Intersect with another. Returns empty if disjoint.
    pub fn intersect(&self, other: &PhysicalBounds) -> Self {
        let x0 = self.origin.x.max(other.origin.x);
        let y0 = self.origin.y.max(other.origin.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= x0 || y1 <= y0 {
            Self::EMPTY
        } else {
            Self::from_xywh(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
        }
    }
}

/// The full virtual desktop bounding box. May have negative minimums when
/// monitors are positioned left of or above the primary display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualBounds {
    /// Inclusive minimum.
    pub min: PhysicalPoint,
    /// Exclusive maximum (right, bottom).
    pub max: PhysicalPoint,
}

impl VirtualBounds {
    /// Total width in physical pixels.
    pub fn width(&self) -> i32 {
        self.max.x - self.min.x
    }

    /// Total height in physical pixels.
    pub fn height(&self) -> i32 {
        self.max.y - self.min.y
    }
}

/// A client-coordinate rectangle (CSS pixels in the WebView). Used to express
/// the overlay stage size so the frontend can convert to physical pixels
/// without owning the conversion logic itself.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientBounds {
    /// Top-left corner in client (CSS) pixels.
    pub origin: ClientPoint,
    /// Extents in client (CSS) pixels.
    pub size: ClientSize,
}

/// A 2D point in client (CSS) pixels. May be fractional.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientPoint {
    /// X in CSS pixels (may be fractional).
    pub x: f64,
    /// Y in CSS pixels (may be fractional).
    pub y: f64,
}

impl ClientPoint {
    /// Convenience constructor.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A 2D size in client (CSS) pixels. May be fractional.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSize {
    /// Width in CSS pixels (may be fractional).
    pub width: f64,
    /// Height in CSS pixels (may be fractional).
    pub height: f64,
}

impl ClientSize {
    /// Convenience constructor.
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

impl ClientBounds {
    /// Empty client bounds at the origin.
    pub const EMPTY: Self = Self {
        origin: ClientPoint::new(0.0, 0.0),
        size: ClientSize::new(0.0, 0.0),
    };

    /// Convenience constructor.
    pub const fn new(origin: ClientPoint, size: ClientSize) -> Self {
        Self { origin, size }
    }

    /// `true` when either axis is at or below zero.
    pub fn is_empty(&self) -> bool {
        self.size.width <= 0.0 || self.size.height <= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_to_i32_handles_non_finite() {
        assert_eq!(transform::round_to_i32(f64::NAN), 0);
        assert_eq!(transform::round_to_i32(f64::INFINITY), 0);
        assert_eq!(transform::round_to_i32(f64::NEG_INFINITY), 0);
    }

    #[test]
    fn round_to_i32_rounds_half_away_from_zero() {
        assert_eq!(transform::round_to_i32(0.5), 1);
        assert_eq!(transform::round_to_i32(-0.5), -1);
        assert_eq!(transform::round_to_i32(1.4), 1);
        assert_eq!(transform::round_to_i32(1.6), 2);
    }

    #[test]
    fn round_to_u32_rejects_negative_or_non_finite() {
        assert_eq!(transform::round_to_u32(f64::NAN), 0);
        assert_eq!(transform::round_to_u32(-1.0), 0);
        assert_eq!(transform::round_to_u32(0.0), 0);
        assert_eq!(transform::round_to_u32(5.5), 6);
    }

    #[test]
    fn round_to_u32_clamps_overflow() {
        let max = u32::MAX as f64;
        assert_eq!(transform::round_to_u32(max + 1.0), u32::MAX);
    }

    #[test]
    fn client_to_physical_zero_stage_is_empty() {
        let capture = PhysicalBounds::from_xywh(0, 0, 1920, 1080);
        let stage = ClientSize::new(0.0, 0.0);
        let client = ClientBounds::new(ClientPoint::new(10.0, 10.0), ClientSize::new(100.0, 100.0));
        assert!(transform::client_to_physical(&client, capture, stage).is_empty());
    }

    #[test]
    fn client_to_physical_empty_selection_is_empty() {
        let capture = PhysicalBounds::from_xywh(0, 0, 1920, 1080);
        let stage = ClientSize::new(960.0, 540.0);
        let client = ClientBounds::EMPTY;
        assert!(transform::client_to_physical(&client, capture, stage).is_empty());
    }

    #[test]
    fn client_to_physical_uses_capture_origin_offset() {
        // Capture buffer does not start at virtual-desktop origin (0, 0);
        // the conversion must still produce the correct physical origin.
        let capture = PhysicalBounds::from_xywh(100, 200, 1920, 1080);
        let stage = ClientSize::new(960.0, 540.0);
        let client =
            ClientBounds::new(ClientPoint::new(120.0, 60.0), ClientSize::new(480.0, 240.0));
        let physical = transform::client_to_physical(&client, capture, stage);
        assert_eq!(physical.origin.x, 340); // 100 + 240
        assert_eq!(physical.origin.y, 320); // 200 + 120
        assert_eq!(physical.size.width, 960);
        assert_eq!(physical.size.height, 480);
    }

    #[test]
    fn physical_to_capture_buffer_subtracts_origin() {
        let physical = PhysicalBounds::from_xywh(50, 60, 30, 40);
        let origin = PhysicalPoint::new(50, 60);
        let result = transform::physical_to_capture_buffer(&physical, origin);
        assert_eq!(result.origin.x, 0);
        assert_eq!(result.origin.y, 0);
        assert_eq!(result.size.width, 30);
        assert_eq!(result.size.height, 40);
    }

    #[test]
    fn physical_to_capture_buffer_clamps_negative() {
        let physical = PhysicalBounds::from_xywh(-5, -10, 30, 40);
        let origin = PhysicalPoint::new(0, 0);
        let result = transform::physical_to_capture_buffer(&physical, origin);
        assert_eq!(result.origin.x, 0);
        assert_eq!(result.origin.y, 0);
    }

    #[test]
    fn clamp_to_capture_buffer_truncates_extent() {
        let crop = PhysicalBounds::from_xywh(0, 0, 100, 100);
        let size = PhysicalSize::new(50, 80);
        let result = transform::clamp_to_capture_buffer(&crop, size);
        assert_eq!(result.size.width, 50);
        assert_eq!(result.size.height, 80);
    }

    #[test]
    fn intersect_returns_empty_when_disjoint() {
        let a = PhysicalBounds::from_xywh(0, 0, 10, 10);
        let b = PhysicalBounds::from_xywh(20, 0, 10, 10);
        assert_eq!(a.intersect(&b), PhysicalBounds::EMPTY);
    }

    #[test]
    fn intersect_returns_overlap() {
        let a = PhysicalBounds::from_xywh(0, 0, 20, 20);
        let b = PhysicalBounds::from_xywh(10, 10, 20, 20);
        let result = a.intersect(&b);
        assert_eq!(result.origin.x, 10);
        assert_eq!(result.origin.y, 10);
        assert_eq!(result.size.width, 10);
        assert_eq!(result.size.height, 10);
    }

    #[test]
    fn clamped_to_extent_caps_dimensions() {
        let inner = PhysicalBounds::from_xywh(0, 0, 100, 100);
        let extent = PhysicalBounds::from_xywh(10, 10, 50, 50);
        let clamped = inner.clamped_to(&extent);
        assert_eq!(clamped.origin.x, 10);
        assert_eq!(clamped.origin.y, 10);
        assert_eq!(clamped.size.width, 50);
        assert_eq!(clamped.size.height, 50);
    }
}

/// Coordinate conversion utilities. Centralised here so the same rounding
/// rules apply at every boundary. See ADR-0003 for the rationale.
pub mod transform {
    use super::{ClientBounds, ClientSize, PhysicalBounds, PhysicalPoint, PhysicalSize};

    /// Round a finite float to the nearest i32. Returns 0 for NaN / infinite
    /// inputs so a bad conversion cannot produce a wildly out-of-range
    /// coordinate.
    pub fn round_to_i32(value: f64) -> i32 {
        if !value.is_finite() {
            return 0;
        }
        // f64::round ties away from zero, which matches the "physical pixel
        // ownership" rule documented in ADR-0003: every fractional component
        // is resolved to the nearest pixel.
        let rounded = value.round();
        if rounded >= i32::MAX as f64 {
            i32::MAX
        } else if rounded <= i32::MIN as f64 {
            i32::MIN
        } else {
            rounded as i32
        }
    }

    /// Round a non-negative finite float to a non-negative u32 size.
    /// Negative or NaN inputs collapse to 0.
    pub fn round_to_u32(value: f64) -> u32 {
        if !value.is_finite() || value <= 0.0 {
            return 0;
        }
        let rounded = value.round();
        if rounded >= u32::MAX as f64 {
            u32::MAX
        } else {
            rounded as u32
        }
    }

    /// Convert a client-coordinate rectangle to a physical-pixel rectangle
    /// in the virtual-desktop coordinate system. The conversion scales by
    /// the ratio between the captured framebuffer's physical extent and the
    /// overlay stage's client extent, then translates by the capture bounds'
    /// origin so the result is expressed in absolute physical pixels. All
    /// four components (origin x, origin y, width, height) are rounded to
    /// the nearest pixel; see `round_to_i32` / `round_to_u32` for the
    /// tie-break rule.
    pub fn client_to_physical(
        client: &ClientBounds,
        capture_bounds: PhysicalBounds,
        stage_size: ClientSize,
    ) -> PhysicalBounds {
        if client.is_empty() || stage_size.width <= 0.0 || stage_size.height <= 0.0 {
            return PhysicalBounds::EMPTY;
        }
        let scale_x = capture_bounds.size.width as f64 / stage_size.width;
        let scale_y = capture_bounds.size.height as f64 / stage_size.height;
        let x = capture_bounds.origin.x + round_to_i32(client.origin.x * scale_x);
        let y = capture_bounds.origin.y + round_to_i32(client.origin.y * scale_y);
        let width = round_to_u32(client.size.width * scale_x);
        let height = round_to_u32(client.size.height * scale_y);
        PhysicalBounds::from_xywh(x, y, width, height)
    }

    /// Translate physical coordinates into the captured framebuffer's local
    /// coordinate space. The framebuffer's top-left is at `capture_origin`
    /// in physical coordinates; the conversion subtracts that origin and
    /// rounds the result to the nearest pixel. Negative results are clamped
    /// to zero so the crop cannot escape the buffer.
    pub fn physical_to_capture_buffer(
        physical: &PhysicalBounds,
        capture_origin: PhysicalPoint,
    ) -> PhysicalBounds {
        let x = round_to_i32(physical.origin.x as f64 - capture_origin.x as f64).max(0);
        let y = round_to_i32(physical.origin.y as f64 - capture_origin.y as f64).max(0);
        let width = round_to_u32(physical.size.width as f64);
        let height = round_to_u32(physical.size.height as f64);
        PhysicalBounds::from_xywh(x, y, width, height)
    }

    /// Ensure a capture-buffer crop stays within the framebuffer's physical
    /// extents. Used as the last guard before exporting.
    pub fn clamp_to_capture_buffer(
        crop: &PhysicalBounds,
        buffer_size: PhysicalSize,
    ) -> PhysicalBounds {
        let extent = PhysicalBounds::from_xywh(0, 0, buffer_size.width, buffer_size.height);
        crop.clamped_to(&extent)
    }
}
