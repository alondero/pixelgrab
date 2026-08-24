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

    #[test]
    fn union_work_area_spans_every_monitor() {
        let mut secondary = desc("secondary", -1920, 0, 1920, 1080, false);
        // Taskbar strip on the secondary too.
        secondary.work_area = PhysicalBounds::from_xywh(-1920, 0, 1920, 1032);
        let layout = MonitorLayout::new(vec![desc("primary", 0, 0, 1920, 1080, true), secondary]);
        let union = layout.union_work_area().expect("union");
        assert_eq!(union.origin.x, -1920);
        assert_eq!(union.origin.y, 0);
        assert_eq!(union.size.width, 3840);
        // The union's bottom edge follows the taller monitor's work area.
        assert_eq!(union.size.height, 1080);
    }

    #[test]
    fn fingerprint_changes_with_geometry_but_is_stable_for_equal_layouts() {
        let a = MonitorLayout::new(vec![desc("primary", 0, 0, 1920, 1080, true)]);
        let b = MonitorLayout::new(vec![desc("primary", 0, 0, 1920, 1080, true)]);
        assert_eq!(a.fingerprint(), b.fingerprint());

        // Resolution change.
        let resized = MonitorLayout::new(vec![desc("primary", 0, 0, 1680, 1050, true)]);
        assert_ne!(a.fingerprint(), resized.fingerprint());

        // Taskbar (work-area) change alone must also register.
        let mut taskbar_changed = desc("primary", 0, 0, 1920, 1080, true);
        taskbar_changed.work_area = PhysicalBounds::from_xywh(0, 0, 1920, 1032);
        let taskbar_layout = MonitorLayout::new(vec![taskbar_changed]);
        assert_ne!(a.fingerprint(), taskbar_layout.fingerprint());

        // DPI change registers.
        let mut dpi_changed = desc("primary", 0, 0, 1920, 1080, true);
        dpi_changed.scale_factor = 1.5;
        assert_ne!(
            a.fingerprint(),
            MonitorLayout::new(vec![dpi_changed]).fingerprint()
        );
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

    /// Bounding box of every monitor's work area. Used by the
    /// display-change hook (issue #63) as the reachable region when
    /// re-anchoring pins; returns `None` on an empty layout.
    pub fn union_work_area(&self) -> Option<PhysicalBounds> {
        let mut iter = self.monitors.iter();
        let first = iter.next()?;
        let mut min_x = first.work_area.origin.x;
        let mut min_y = first.work_area.origin.y;
        let mut max_x = first.work_area.right();
        let mut max_y = first.work_area.bottom();
        for m in iter {
            min_x = min_x.min(m.work_area.origin.x);
            min_y = min_y.min(m.work_area.origin.y);
            max_x = max_x.max(m.work_area.right());
            max_y = max_y.max(m.work_area.bottom());
        }
        Some(PhysicalBounds::from_xywh(
            min_x,
            min_y,
            (max_x - min_x).max(0) as u32,
            (max_y - min_y).max(0) as u32,
        ))
    }

    /// Stable fingerprint of the geometry-relevant layout state:
    /// every monitor's id, bounds, scale factor, and work area, in
    /// order, folded through FNV-1a 64-bit. The display watcher uses
    /// this to detect topology / resolution / DPI / work-area changes
    /// without shipping full layouts across the comparison.
    pub fn fingerprint(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        let mut hash = FNV_OFFSET;
        fn fold(hash: &mut u64, bytes: &[u8]) {
            for byte in bytes {
                *hash ^= u64::from(*byte);
                *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        for m in &self.monitors {
            fold(&mut hash, m.id.as_bytes());
            fold(&mut hash, &m.bounds.origin.x.to_le_bytes());
            fold(&mut hash, &m.bounds.origin.y.to_le_bytes());
            fold(&mut hash, &m.bounds.size.width.to_le_bytes());
            fold(&mut hash, &m.bounds.size.height.to_le_bytes());
            fold(&mut hash, &m.scale_factor.to_le_bytes());
            fold(&mut hash, &m.work_area.origin.x.to_le_bytes());
            fold(&mut hash, &m.work_area.origin.y.to_le_bytes());
            fold(&mut hash, &m.work_area.size.width.to_le_bytes());
            fold(&mut hash, &m.work_area.size.height.to_le_bytes());
            fold(&mut hash, &[u8::from(m.is_primary)]);
        }
        hash
    }
}
