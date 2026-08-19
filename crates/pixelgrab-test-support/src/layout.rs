//! Deterministic synthetic monitor layouts.

use pixelgrab_contracts::{
    coordinate::{PhysicalBounds, PhysicalSize},
    monitor::{MonitorDescriptor, MonitorLayout},
};

/// A factory for monitor layouts that tests can rely on.
#[derive(Debug, Clone)]
pub struct SyntheticMonitorLayout;

impl SyntheticMonitorLayout {
    /// Single primary monitor at origin, 1920x1080, 100% scale.
    pub fn single_primary() -> MonitorLayout {
        MonitorLayout::new(vec![Self::primary(
            "monitor-0",
            "Test Primary",
            PhysicalBounds::from_xywh(0, 0, 1920, 1080),
            1.0,
        )])
    }

    /// Two side-by-side monitors, primary on the left.
    /// First is 1920x1080, second is 2560x1440, 100% scale.
    pub fn dual_side_by_side() -> MonitorLayout {
        MonitorLayout::new(vec![
            Self::primary(
                "monitor-0",
                "Test Primary",
                PhysicalBounds::from_xywh(0, 0, 1920, 1080),
                1.0,
            ),
            Self::secondary(
                "monitor-1",
                "Test Secondary",
                PhysicalBounds::from_xywh(1920, 0, 2560, 1440),
                1.0,
            ),
        ])
    }

    /// Negative-origin layout: secondary sits left of and above the primary.
    /// Total virtual bounds are 4480 x 2160 with min=(1920, -200).
    pub fn dual_negative_origin() -> MonitorLayout {
        MonitorLayout::new(vec![
            Self::primary(
                "monitor-0",
                "Test Primary",
                PhysicalBounds::from_xywh(1920, 0, 2560, 1440),
                1.0,
            ),
            Self::secondary(
                "monitor-1",
                "Test Secondary",
                PhysicalBounds::from_xywh(0, -200, 1920, 1080),
                1.0,
            ),
        ])
    }

    /// Mixed-DPI layout: 100% and 125% on the same virtual desktop.
    pub fn mixed_dpi() -> MonitorLayout {
        MonitorLayout::new(vec![
            Self::primary(
                "monitor-0",
                "Test Primary",
                PhysicalBounds::from_xywh(0, 0, 1920, 1080),
                1.0,
            ),
            Self::secondary(
                "monitor-1",
                "Test Secondary",
                PhysicalBounds::from_xywh(1920, 0, 1920, 1080),
                1.25,
            ),
        ])
    }

    /// Stacked layout: primary on top, secondary directly below. Tests
    /// vertical monitor arrangements.
    pub fn stacked() -> MonitorLayout {
        MonitorLayout::new(vec![
            Self::primary(
                "monitor-0",
                "Test Primary",
                PhysicalBounds::from_xywh(0, 0, 1920, 1080),
                1.0,
            ),
            Self::secondary(
                "monitor-1",
                "Test Secondary",
                PhysicalBounds::from_xywh(0, 1080, 1920, 1080),
                1.0,
            ),
        ])
    }

    /// Vertically offset layout: secondary hangs above the primary by
    /// 200 px. Exercises the vertical negative-origin code path.
    pub fn vertically_offset() -> MonitorLayout {
        MonitorLayout::new(vec![
            Self::primary(
                "monitor-0",
                "Test Primary",
                PhysicalBounds::from_xywh(0, 0, 1920, 1080),
                1.0,
            ),
            Self::secondary(
                "monitor-1",
                "Test Secondary",
                PhysicalBounds::from_xywh(0, -200, 1920, 1080),
                1.0,
            ),
        ])
    }

    /// Overlapping-edge layout: secondary overlaps the primary by 100 px
    /// so the seam is not at a clean monitor boundary. Useful for the
    /// acceptance criterion "DPI boundaries do not produce selection or
    /// annotation jumps".
    pub fn overlapping_edge() -> MonitorLayout {
        MonitorLayout::new(vec![
            Self::primary(
                "monitor-0",
                "Test Primary",
                PhysicalBounds::from_xywh(0, 0, 1920, 1080),
                1.0,
            ),
            Self::secondary(
                "monitor-1",
                "Test Secondary",
                PhysicalBounds::from_xywh(1820, 0, 1920, 1080),
                1.0,
            ),
        ])
    }

    /// 100% / 200% mixed-DPI layout. The 200% monitor reports
    /// physical pixels that are 2x the logical pixels, so the framework
    /// has to honour the physical extent regardless of DPI.
    pub fn mixed_dpi_200() -> MonitorLayout {
        MonitorLayout::new(vec![
            Self::primary(
                "monitor-0",
                "Test Primary",
                PhysicalBounds::from_xywh(0, 0, 1920, 1080),
                1.0,
            ),
            Self::secondary(
                "monitor-1",
                "Test Secondary",
                PhysicalBounds::from_xywh(1920, 0, 3840, 2160),
                2.0,
            ),
        ])
    }

    /// Three-monitor layout: primary in the centre with secondaries
    /// left and right. Exercises the fan-out path with more than two
    /// capture workers.
    pub fn tri_monitor() -> MonitorLayout {
        MonitorLayout::new(vec![
            Self::secondary(
                "monitor-0",
                "Test Left",
                PhysicalBounds::from_xywh(-1920, 0, 1920, 1080),
                1.0,
            ),
            Self::primary(
                "monitor-1",
                "Test Primary",
                PhysicalBounds::from_xywh(0, 0, 1920, 1080),
                1.0,
            ),
            Self::secondary(
                "monitor-2",
                "Test Right",
                PhysicalBounds::from_xywh(1920, 0, 1920, 1080),
                1.0,
            ),
        ])
    }

    /// Build a layout from a list of `(id, x, y, w, h, scale, primary)`
    /// tuples. Public hook for table-driven tests that want to enumerate
    /// many permutations without a dedicated fixture.
    #[allow(clippy::too_many_arguments)]
    pub fn arbitrary(monitors: &[(&str, i32, i32, u32, u32, f32, bool)]) -> MonitorLayout {
        let descriptors = monitors
            .iter()
            .map(|(id, x, y, w, h, scale, primary)| {
                if *primary {
                    Self::primary(id, id, PhysicalBounds::from_xywh(*x, *y, *w, *h), *scale)
                } else {
                    Self::secondary(id, id, PhysicalBounds::from_xywh(*x, *y, *w, *h), *scale)
                }
            })
            .collect();
        MonitorLayout::new(descriptors)
    }

    fn primary(id: &str, label: &str, bounds: PhysicalBounds, scale: f32) -> MonitorDescriptor {
        MonitorDescriptor {
            id: id.to_string(),
            label: label.to_string(),
            is_primary: true,
            bounds,
            scale_factor: scale,
            work_area: bounds,
        }
    }

    fn secondary(id: &str, label: &str, bounds: PhysicalBounds, scale: f32) -> MonitorDescriptor {
        MonitorDescriptor {
            id: id.to_string(),
            label: label.to_string(),
            is_primary: false,
            bounds,
            scale_factor: scale,
            work_area: bounds,
        }
    }

    /// Compute the total virtual bounds for a layout.
    pub fn virtual_bounds(layout: &MonitorLayout) -> (i32, i32, i32, i32) {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for m in &layout.monitors {
            min_x = min_x.min(m.bounds.origin.x);
            min_y = min_y.min(m.bounds.origin.y);
            max_x = max_x.max(m.bounds.right());
            max_y = max_y.max(m.bounds.bottom());
        }
        (min_x, min_y, max_x, max_y)
    }

    /// Compute the total virtual size for a layout.
    pub fn virtual_size(layout: &MonitorLayout) -> PhysicalSize {
        let (min_x, min_y, max_x, max_y) = Self::virtual_bounds(layout);
        PhysicalSize::new((max_x - min_x) as u32, (max_y - min_y) as u32)
    }
}
