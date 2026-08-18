//! Monitor layout descriptors. See ADR-0003 for physical-coordinate ownership.

use serde::{Deserialize, Serialize};

use crate::coordinate::{PhysicalBounds, PhysicalPoint, PhysicalSize};

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
}
