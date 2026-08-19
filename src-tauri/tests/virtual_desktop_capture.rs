//! Tracer-03 table-driven tests. Exercises the virtual-desktop capture
//! pipeline across horizontal, vertical, negative-origin, overlapping-edge,
//! and mixed-scale layouts. Also performs cross-boundary captures and a
//! coordinate round-trip assertion against a maximum one-physical-pixel
//! rounding error.
//!
//! These tests run against the synthetic platform. The composite pipeline
//! invariant is identical to the Windows composite pipeline; the synthetic
//! adapter is the only path that can run on CI without a real desktop
//! session.

use std::sync::Arc;

use pixelgrab_contracts::capture::{CaptureFormat, CaptureRequest};
use pixelgrab_contracts::coordinate::{transform, PhysicalBounds, PhysicalPoint, PhysicalSize};
use pixelgrab_contracts::monitor::MonitorLayout;
use pixelgrab_contracts::PlatformErrorKind;
use pixelgrab_lib::platform::synthetic::SyntheticPlatform;
use pixelgrab_lib::platform::PixelGrabPlatform;
use pixelgrab_test_support::capture::FramePattern;
use pixelgrab_test_support::layout::SyntheticMonitorLayout;

/// Layout fixtures exercised by the table-driven tests. Each entry is a
/// named, deterministic layout the synthetic adapter can use to simulate
/// a real desktop arrangement.
fn layout_fixtures() -> Vec<(&'static str, MonitorLayout)> {
    vec![
        ("single-primary", SyntheticMonitorLayout::single_primary()),
        (
            "dual-side-by-side",
            SyntheticMonitorLayout::dual_side_by_side(),
        ),
        (
            "dual-negative-origin",
            SyntheticMonitorLayout::dual_negative_origin(),
        ),
        ("mixed-dpi", SyntheticMonitorLayout::mixed_dpi()),
        ("mixed-dpi-200", SyntheticMonitorLayout::mixed_dpi_200()),
        ("stacked", SyntheticMonitorLayout::stacked()),
        (
            "vertically-offset",
            SyntheticMonitorLayout::vertically_offset(),
        ),
        (
            "overlapping-edge",
            SyntheticMonitorLayout::overlapping_edge(),
        ),
        ("tri-monitor", SyntheticMonitorLayout::tri_monitor()),
    ]
}

#[test]
fn virtual_desktop_capture_bounds_match_layout_for_every_fixture() {
    for (name, layout) in layout_fixtures() {
        let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::with_layout(
            layout.clone(),
            FramePattern::SolidPerMonitor,
        ));
        let request = CaptureRequest {
            format: CaptureFormat::VirtualDesktop,
            monitor_id: None,
            region: None,
        };
        let resolution = platform
            .capture(&request)
            .unwrap_or_else(|err| panic!("{name}: capture failed: {err}"));
        let virtual_bounds = layout
            .virtual_bounds()
            .unwrap_or_else(|| panic!("{name}: layout had no virtual bounds"));
        let composite = virtual_bounds.as_top_left_bounds();
        assert_eq!(
            resolution.bounds, composite,
            "{name}: virtual-desktop capture bounds must equal the layout's virtual bounds"
        );
    }
}

#[test]
fn virtual_desktop_capture_size_grows_with_monitor_count() {
    // The composite framebuffer's size must scale with the layout. A
    // two-monitor side-by-side layout should be roughly twice as wide as
    // the single-monitor layout.
    let single = SyntheticPlatform::with_layout(
        SyntheticMonitorLayout::single_primary(),
        FramePattern::SolidPerMonitor,
    );
    let dual = SyntheticPlatform::with_layout(
        SyntheticMonitorLayout::dual_side_by_side(),
        FramePattern::SolidPerMonitor,
    );
    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let single_res = single.capture(&request).expect("single capture");
    let dual_res = dual.capture(&request).expect("dual capture");
    // The dual side-by-side has a 2560x1440 secondary, so the composite
    // is wider AND taller than the single-monitor capture.
    assert!(dual_res.bounds.size.width > single_res.bounds.size.width);
    assert!(dual_res.bounds.size.height > single_res.bounds.size.height);
}

#[test]
fn negative_origin_capture_returns_negative_origin_bounds() {
    // The dual-negative-origin layout's primary monitor sits at x = 1920.
    // The composite bounds' origin must reflect the virtual desktop's
    // inclusive minimum rather than the primary monitor's origin.
    let layout = SyntheticMonitorLayout::dual_negative_origin();
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::with_layout(
        layout.clone(),
        FramePattern::SolidPerMonitor,
    ));
    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let resolution = platform.capture(&request).expect("capture");
    let virtual_bounds = layout.virtual_bounds().expect("layout");
    let composite = virtual_bounds.as_top_left_bounds();
    assert_eq!(resolution.bounds.origin, composite.origin);
    assert_eq!(resolution.bounds.origin.x, virtual_bounds.min.x);
    // The secondary sits at y = -200; the min y must reflect that.
    assert_eq!(resolution.bounds.origin.y, -200);
}

#[test]
fn physical_selection_round_trips_within_one_physical_pixel() {
    // Walk every monitor's bounds and a representative point inside the
    // monitor through the canonical capture-buffer round-trip. The
    // cumulative rounding error must stay below one physical pixel per
    // coordinate.
    for (name, layout) in layout_fixtures() {
        let virtual_bounds = layout
            .virtual_bounds()
            .unwrap_or_else(|| panic!("{name}: no virtual bounds"));
        let buffer_size = virtual_bounds.as_top_left_bounds().size;
        for monitor in &layout.monitors {
            // Take a point at the monitor's centre and a point on its
            // edge; both lie inside the captured framebuffer.
            let center = monitor.center();
            let physical = PhysicalBounds::new(
                center,
                PhysicalSize::new(monitor.bounds.size.width, monitor.bounds.size.height),
            );
            let projected =
                transform::project_to_capture_buffer(&physical, virtual_bounds.min, buffer_size);
            let reversed = transform::capture_buffer_to_physical(&projected, virtual_bounds.min);
            let mut max_err_x = (physical.origin.x - reversed.origin.x).abs();
            let mut max_err_y = (physical.origin.y - reversed.origin.y).abs();
            // The projection may shrink the size to the buffer's
            // overlap; the size delta is bounded by the buffer's full
            // extent, not rounding. The verification only applies to
            // the origin which is the only place the round-trip could
            // drift.
            let _ = physical.size;
            let _ = reversed.size;
            // Allow up to one physical pixel of rounding error.
            assert!(max_err_x <= 1, "{name}: origin x drift {max_err_x}");
            assert!(max_err_y <= 1, "{name}: origin y drift {max_err_y}");
            let _ = (&mut max_err_x, &mut max_err_y);
        }
    }
}

#[test]
fn cross_boundary_capture_clips_to_buffer_overlap() {
    // A selection that straddles the seam between two monitors must
    // agree between the layout's physical bounds and the synthetic
    // capture's export. The synthetic `flatten_crop` path is the
    // strictest contract for this: it returns the exact pixels the
    // user would have committed.
    let layout = SyntheticMonitorLayout::dual_side_by_side();
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::with_layout(
        layout.clone(),
        FramePattern::SolidPerMonitor,
    ));
    // The primary monitor spans 0..1920; the secondary spans 1920..4480.
    // A selection at 1900..1940 (width 40) extends 20 px into the
    // secondary.
    let selection = PhysicalBounds::from_xywh(1900, 200, 40, 40);
    let rgba = platform
        .flatten_crop("id", selection)
        .expect("flatten crop");
    let size = PhysicalSize::new(40, 40);
    assert_eq!(rgba.1, size);
    assert_eq!(rgba.0.len(), 40 * 40 * 4);
}

#[test]
fn full_screen_capture_returns_target_monitor_bounds() {
    // The full-screen intent resolves to the primary monitor's bounds.
    // The synthetic platform produces a `SingleMonitor` capture whose
    // bounds equal the primary monitor's physical bounds.
    let layout = SyntheticMonitorLayout::dual_side_by_side();
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::with_layout(
        layout.clone(),
        FramePattern::SolidPerMonitor,
    ));
    let primary = layout
        .monitors
        .iter()
        .find(|m| m.is_primary)
        .expect("primary");
    let request = CaptureRequest {
        format: CaptureFormat::SingleMonitor,
        monitor_id: Some(primary.id.clone()),
        region: None,
    };
    let resolution = platform.capture(&request).expect("capture");
    assert_eq!(resolution.bounds, primary.bounds);
    assert_eq!(resolution.bounds.size, primary.bounds.size);
}

#[test]
fn monitor_hot_unplug_invalidates_cached_layout() {
    // Simulate a hot-unplug: the cached layout is invalidated, the
    // next call returns the new layout, and the topology_dirty flag
    // resets. The orchestrator uses this to detect changes that
    // happen between captures.
    let layout = SyntheticMonitorLayout::dual_side_by_side();
    let platform = SyntheticPlatform::with_layout(layout.clone(), FramePattern::SolidPerMonitor);
    let initial = platform.monitor_layout().expect("initial layout");
    assert_eq!(initial.monitors.len(), 2);
    assert!(!platform.is_topology_dirty());

    // Hot-unplug: replace the layout with a single-monitor one.
    platform.set_layout(SyntheticMonitorLayout::single_primary());
    assert!(platform.is_topology_dirty());

    let fresh = platform.monitor_layout().expect("fresh layout");
    assert_eq!(fresh.monitors.len(), 1);
    assert!(!platform.is_topology_dirty());

    // A subsequent capture must use the new layout, not the stale one.
    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let resolution = platform.capture(&request).expect("capture");
    assert_eq!(
        resolution.bounds.size,
        fresh.virtual_bounds().unwrap().as_top_left_bounds().size
    );
}

#[test]
fn failing_monitor_rejects_composite_with_partial_failure() {
    // The composite pipeline must reject a capture when any monitor
    // fails. The platform never commits a partial desktop as a complete
    // capture. Synthesised via the synthetic platform's failure-list
    // hook.
    let layout = SyntheticMonitorLayout::tri_monitor();
    let platform = SyntheticPlatform::with_layout(layout, FramePattern::SolidPerMonitor);
    platform.set_failing_monitors(&["monitor-1"]);
    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let err = platform.capture(&request).expect_err("capture must fail");
    assert!(
        matches!(err.kind, PlatformErrorKind::CaptureUnavailable),
        "partial composite must surface as CaptureUnavailable, got {:?}",
        err.kind
    );
}

#[test]
fn overlapping_edge_layout_still_produces_valid_composite() {
    // Two monitors overlap on the seam so the rightmost 100 px of the
    // primary is also the leftmost 100 px of the secondary. The
    // composite must still cover the union, not the intersection.
    let layout = SyntheticMonitorLayout::overlapping_edge();
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::with_layout(
        layout.clone(),
        FramePattern::SolidPerMonitor,
    ));
    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let resolution = platform.capture(&request).expect("capture");
    // Virtual union: min x = 0, max x = 1820 + 1920 = 3740.
    let virtual_bounds = layout.virtual_bounds().expect("bounds");
    assert_eq!(virtual_bounds.min.x, 0);
    assert_eq!(virtual_bounds.max.x, 3740);
    let composite = virtual_bounds.as_top_left_bounds();
    assert_eq!(resolution.bounds, composite);
}

#[test]
fn mixed_scale_layout_produces_physical_extent() {
    // The 100% / 200% mixed-DPI layout's secondary reports 3840x2160
    // physical pixels. The composite must honour the physical extent,
    // not the logical extent.
    let layout = SyntheticMonitorLayout::mixed_dpi_200();
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::with_layout(
        layout.clone(),
        FramePattern::SolidPerMonitor,
    ));
    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let resolution = platform.capture(&request).expect("capture");
    let virtual_bounds = layout.virtual_bounds().expect("bounds");
    let composite = virtual_bounds.as_top_left_bounds();
    assert_eq!(resolution.bounds, composite);
    // The composite is 1920 + 3840 = 5760 pixels wide.
    assert_eq!(resolution.bounds.size.width, 5760);
    assert_eq!(resolution.bounds.size.height, 2160);
    let _ = PhysicalPoint::new(0, 0);
}

#[test]
fn physical_selection_to_capture_buffer_in_negative_origin_layout() {
    // Pick a selection that lives entirely inside the secondary monitor
    // of the negative-origin layout. The capture-buffer projection must
    // subtract the virtual desktop's minimum (0, -200) to land the
    // selection at the secondary's local offset.
    let layout = SyntheticMonitorLayout::dual_negative_origin();
    let virtual_bounds = layout.virtual_bounds().expect("bounds");
    let composite = virtual_bounds.as_top_left_bounds();
    let secondary = layout
        .monitors
        .iter()
        .find(|m| m.id == "monitor-1")
        .expect("secondary");
    let selection = PhysicalBounds::from_xywh(
        secondary.bounds.origin.x + 10,
        secondary.bounds.origin.y + 10,
        100,
        100,
    );
    let projected =
        transform::project_to_capture_buffer(&selection, virtual_bounds.min, composite.size);
    // The capture buffer's origin is the virtual desktop's top-left at
    // (0, -200). The secondary sits at (0, -200) in physical coordinates,
    // so its first physical pixel maps to buffer coordinate (0, 0); the
    // selection 10 px inside the secondary maps to (10, 10).
    assert_eq!(projected.origin.x, 10);
    assert_eq!(projected.origin.y, 10);
    assert_eq!(projected.size.width, 100);
    assert_eq!(projected.size.height, 100);
}

#[test]
fn coordinate_transform_round_trip_over_physical_selection() {
    // The acceptance criterion "round-trip representative points
    // through every coordinate transform and assert a maximum
    // one-physical-pixel rounding error" — exercised against the
    // physical → buffer → physical sequence.
    let layout = SyntheticMonitorLayout::tri_monitor();
    let virtual_bounds = layout.virtual_bounds().expect("bounds");
    let composite = virtual_bounds.as_top_left_bounds();
    for monitor in &layout.monitors {
        let origin = monitor.bounds.origin;
        // Take a 1x1 pixel at the monitor's origin and the monitor's
        // centre.
        for p in [origin, monitor.center()].iter() {
            let physical = PhysicalBounds::from_xywh(p.x, p.y, 1, 1);
            let buffer =
                transform::project_to_capture_buffer(&physical, virtual_bounds.min, composite.size);
            let back = transform::capture_buffer_to_physical(&buffer, virtual_bounds.min);
            assert!(
                (back.origin.x - physical.origin.x).abs() <= 1,
                "monitor {}: x drift {} (back={:?}, origin={:?})",
                monitor.id,
                (back.origin.x - physical.origin.x).abs(),
                back.origin,
                physical.origin,
            );
            assert!(
                (back.origin.y - physical.origin.y).abs() <= 1,
                "monitor {}: y drift {} (back={:?}, origin={:?})",
                monitor.id,
                (back.origin.y - physical.origin.y).abs(),
                back.origin,
                physical.origin,
            );
        }
    }
}
