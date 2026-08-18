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
