//! Monitor layout descriptors. See ADR-0003 for physical-coordinate ownership.

use serde::{Deserialize, Serialize};

use crate::coordinate::{PhysicalBounds, PhysicalPoint, PhysicalSize, VirtualBounds};

/// Stable descriptor for a single display. The `id` survives topology changes
/// for the same physical monitor; it is implementation-defined but stable per
/// session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDescriptor {
    /// Stable identifier.
    pub id: String,
    /// Human-readable label (e.g. "DELL U2719DX").
    pub label: String,
    /// Whether this is the primary display.
    pub is_primary: bool,
    /// Physical bounds in the virtual desktop coordinate system.
    pub bounds: PhysicalBounds,
    /// Per-monitor scale factor at 100% (e.g. 1.0, 1.25, 1.5, 2.0).
    pub scale_factor: f32,
    /// Monotonically increasing work-area inset (e.g. taskbar).
    pub work_area: PhysicalBounds,
}

impl MonitorDescriptor {
    /// Convenience: physical center of the monitor.
    pub fn center(&self) -> PhysicalPoint {
        PhysicalPoint::new(
            self.bounds.origin.x + (self.bounds.size.width as i32) / 2,
            self.bounds.origin.y + (self.bounds.size.height as i32) / 2,
        )
    }

    /// Convenience: physical size.
    pub fn size(&self) -> PhysicalSize {
        self.bounds.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(id: &str, x: i32, y: i32, w: u32, h: u32, primary: bool) -> MonitorDescriptor {
        MonitorDescriptor {
            id: id.to_string(),
            label: id.to_string(),
            is_primary: primary,
            bounds: PhysicalBounds::from_xywh(x, y, w, h),
            scale_factor: 1.0,
            work_area: PhysicalBounds::from_xywh(x, y, w, h),
        }
    }

    #[test]
    fn virtual_bounds_handles_negative_origin() {
        // Secondary above and left of the primary; min=(0, -200).
        let layout = MonitorLayout::new(vec![
            desc("primary", 1920, 0, 2560, 1440, true),
            desc("secondary", 0, -200, 1920, 1080, false),
        ]);
        let v = layout.virtual_bounds().expect("bounds");
        assert_eq!(v.min, PhysicalPoint::new(0, -200));
        // Primary's right edge is 1920 + 2560 = 4480; max y is 1440.
        assert_eq!(v.max, PhysicalPoint::new(4480, 1440));
        assert_eq!(v.width(), 4480);
        assert_eq!(v.height(), 1640);
    }

    #[test]
    fn virtual_bounds_returns_none_on_empty_layout() {
        let layout = MonitorLayout::new(vec![]);
        assert!(layout.virtual_bounds().is_none());
    }

    #[test]
    fn primary_returns_nothing_when_no_primary_flag() {
        let layout = MonitorLayout::new(vec![desc("a", 0, 0, 100, 100, false)]);
        assert!(layout.primary().is_none());
    }
}

/// Ordered list of monitors.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorLayout {
    /// All monitors currently visible to the OS.
    pub monitors: Vec<MonitorDescriptor>,
}

impl MonitorLayout {
    /// Construct from a list of monitors. The primary monitor is assumed to
    /// be at index 0.
    pub fn new(monitors: Vec<MonitorDescriptor>) -> Self {
        Self { monitors }
    }

    /// Returns the primary monitor, if any.
    pub fn primary(&self) -> Option<&MonitorDescriptor> {
        self.monitors.iter().find(|m| m.is_primary)
    }

    /// Returns the monitor containing the given physical point, if any.
    pub fn monitor_containing(&self, point: PhysicalPoint) -> Option<&MonitorDescriptor> {
        self.monitors.iter().find(|m| {
            m.bounds.origin.x <= point.x
                && point.x < m.bounds.right()
                && m.bounds.origin.y <= point.y
                && point.y < m.bounds.bottom()
        })
    }

    /// Total virtual desktop bounds (inclusive min, exclusive max). Returns
    /// `None` when the layout is empty: the capture pipeline cannot accept
    /// a "no monitors" state and the overlay window has nothing to cover.
    pub fn virtual_bounds(&self) -> Option<VirtualBounds> {
        if self.monitors.is_empty() {
            return None;
        }
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for m in &self.monitors {
            min_x = min_x.min(m.bounds.origin.x);
            min_y = min_y.min(m.bounds.origin.y);
            max_x = max_x.max(m.bounds.right());
            max_y = max_y.max(m.bounds.bottom());
        }
        Some(VirtualBounds {
            min: PhysicalPoint::new(min_x, min_y),
            max: PhysicalPoint::new(max_x, max_y),
        })
    }
}
