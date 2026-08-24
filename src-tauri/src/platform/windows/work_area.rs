//! Real Windows monitor work-area discovery (issue #63).
//!
//! `xcap` reports each monitor's full bounds but not its *work area*
//! (the bounds minus the taskbar and application bars). Without real
//! work areas the shelf placement math can drop cards behind the
//! taskbar, so this module queries `GetMonitorInfoW` through a minimal
//! hand-rolled FFI surface — matching the repo convention of avoiding
//! a dependency on the `windows` crate's macro evolution.
//!
//! The geometry merge ([`apply_work_areas`]) is a pure,
//! platform-independent function so the test suite can exercise the
//! matching policy without a live desktop.

use pixelgrab_contracts::MonitorLayout;

/// Win32 `RECT`.
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// Raw per-monitor geometry as reported by `GetMonitorInfoW`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMonitorArea {
    /// Full monitor rect (`rcMonitor`): left, top, right, bottom.
    pub monitor: (i32, i32, i32, i32),
    /// Work-area rect (`rcWork`): left, top, right, bottom.
    pub work: (i32, i32, i32, i32),
}

/// Merge raw OS work areas into a layout's descriptors.
///
/// Each descriptor is matched against the raw list by its physical
/// origin (`x`, `y`) — origins are unique across a virtual desktop.
/// A matched descriptor gets its real `work_area`; an unmatched one
/// keeps its full bounds as a conservative fallback (the shelf may
/// overlap the taskbar rather than fail outright). The returned layout
/// preserves input order so callers cannot accidentally re-order the
/// monitor list by calling this function.
pub fn apply_work_areas(layout: &MonitorLayout, raw: &[RawMonitorArea]) -> MonitorLayout {
    let mut monitors = layout.monitors.clone();
    for descriptor in &mut monitors {
        if let Some(area) = raw.iter().find(|r| {
            r.monitor.0 == descriptor.bounds.origin.x && r.monitor.1 == descriptor.bounds.origin.y
        }) {
            let (left, top, right, bottom) = area.work;
            let width = (right - left).max(0) as u32;
            let height = (bottom - top).max(0) as u32;
            if width > 0 && height > 0 {
                descriptor.work_area =
                    pixelgrab_contracts::PhysicalBounds::from_xywh(left, top, width, height);
            }
        }
    }
    MonitorLayout::new(monitors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixelgrab_contracts::{MonitorDescriptor, PhysicalBounds};

    fn descriptor(id: &str, x: i32, y: i32, w: u32, h: u32) -> MonitorDescriptor {
        MonitorDescriptor {
            id: id.to_string(),
            label: id.to_string(),
            is_primary: id == "m1",
            bounds: PhysicalBounds::from_xywh(x, y, w, h),
            scale_factor: 1.0,
            work_area: PhysicalBounds::from_xywh(x, y, w, h),
        }
    }

    #[test]
    fn apply_work_areas_matches_by_origin_and_preserves_order() {
        let layout = MonitorLayout::new(vec![
            descriptor("m1", 0, 0, 1920, 1080),
            descriptor("m2", -1920, 0, 1920, 1080),
        ]);
        let raw = vec![
            RawMonitorArea {
                monitor: (-1920, 0, 0, 1080),
                work: (-1920, 0, 0, 1032),
            },
            RawMonitorArea {
                monitor: (0, 0, 1920, 1080),
                work: (0, 0, 1920, 1032),
            },
        ];
        let merged = apply_work_areas(&layout, &raw);
        assert_eq!(merged.monitors[0].id, "m1");
        assert_eq!(merged.monitors[1].id, "m2");
        // Primary keeps its position; taskbar strip removed from height.
        assert_eq!(merged.monitors[0].work_area.size.height, 1032);
        assert_eq!(merged.monitors[0].work_area.size.width, 1920);
        assert_eq!(merged.monitors[1].work_area.size.height, 1032);
        // Full bounds are untouched.
        assert_eq!(merged.monitors[1].bounds.size.height, 1080);
    }

    #[test]
    fn unmatched_descriptors_keep_full_bounds() {
        let layout = MonitorLayout::new(vec![descriptor("m1", 0, 0, 1920, 1080)]);
        let merged = apply_work_areas(&layout, &[]);
        assert_eq!(merged.monitors[0].work_area, merged.monitors[0].bounds);
    }

    #[test]
    fn degenerate_work_rects_are_ignored() {
        let layout = MonitorLayout::new(vec![descriptor("m1", 0, 0, 1920, 1080)]);
        let raw = vec![RawMonitorArea {
            monitor: (0, 0, 1920, 1080),
            // Zero-height work area would strand every window; ignore it.
            work: (0, 0, 1920, 0),
        }];
        let merged = apply_work_areas(&layout, &raw);
        assert_eq!(merged.monitors[0].work_area, merged.monitors[0].bounds);
    }
}

// ---------------------------------------------------------------------------
// Windows FFI. Compiled only on Windows targets; the pure merge above is
// what the test suite exercises everywhere.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub(crate) mod ffi {
    use super::{RawMonitorArea, Rect};
    use std::sync::Mutex;

    type Hmonitor = isize;
    type Lparam = isize;
    type Bool = i32;
    type MonitorEnumProc =
        Option<unsafe extern "system" fn(Hmonitor, Hmonitor, *mut Rect, Lparam) -> Bool>;

    #[repr(C)]
    struct MonitorInfoW {
        cb_size: u32,
        rc_monitor: Rect,
        rc_work: Rect,
        dw_flags: u32,
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumDisplayMonitors(
            hdc: isize,
            lprc_clip: *const Rect,
            lpfn_enum: MonitorEnumProc,
            dw_data: Lparam,
        ) -> Bool;
        fn GetMonitorInfoW(hmonitor: Hmonitor, lpmi: *mut MonitorInfoW) -> Bool;
        fn GetCursorPos(lp_point: *mut Point) -> Bool;
    }

    static COLLECTED: Mutex<Vec<RawMonitorArea>> = Mutex::new(Vec::new());

    /// SAFETY: Windows callback contract — `lprm_monitor` is a valid
    /// `HMONITOR` for the duration of the call and `lparam` carries the
    /// pointer we passed to `EnumDisplayMonitors`. We only read Win32
    /// structs and push into our own mutex-guarded vector.
    unsafe extern "system" fn enum_callback(
        _hdc: Hmonitor,
        hmonitor: Hmonitor,
        _rect: *mut Rect,
        _lparam: Lparam,
    ) -> Bool {
        let mut info = MonitorInfoW {
            cb_size: std::mem::size_of::<MonitorInfoW>() as u32,
            rc_monitor: Rect::default(),
            rc_work: Rect::default(),
            dw_flags: 0,
        };
        if GetMonitorInfoW(hmonitor, &mut info) != 0 {
            let area = RawMonitorArea {
                monitor: (
                    info.rc_monitor.left,
                    info.rc_monitor.top,
                    info.rc_monitor.right,
                    info.rc_monitor.bottom,
                ),
                work: (
                    info.rc_work.left,
                    info.rc_work.top,
                    info.rc_work.right,
                    info.rc_work.bottom,
                ),
            };
            if let Ok(mut collected) = COLLECTED.lock() {
                collected.push(area);
            }
        }
        1 // continue enumeration
    }

    /// Query every monitor's full + work rects from the OS.
    pub fn query_raw_work_areas() -> Vec<RawMonitorArea> {
        if let Ok(mut collected) = COLLECTED.lock() {
            collected.clear();
        }
        // SAFETY: no HDC required (null passes the desktop); the enum
        // callback above follows the documented contract.
        unsafe {
            EnumDisplayMonitors(0, std::ptr::null(), Some(enum_callback), 0);
        }
        COLLECTED.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Current cursor position in physical desktop coordinates, or
    /// `None` when the interactive desktop is unavailable.
    pub fn query_cursor_position() -> Option<(i32, i32)> {
        let mut point = Point { x: 0, y: 0 };
        // SAFETY: `GetCursorPos` writes exactly one `POINT`.
        let ok = unsafe { GetCursorPos(&mut point) };
        if ok == 0 {
            None
        } else {
            Some((point.x, point.y))
        }
    }
}
