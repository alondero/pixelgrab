//! Pre-allocated overlay lifecycle. The overlay is built and hidden during
//! application setup so the first capture does not pay a window-creation
//! cost.
//!
//! Once a capture is committed, the overlay window is positioned over the
//! virtual desktop so the freeze frame covers every active monitor. The
//! bounds are stored in physical pixels: the overlay's Konva stage fills
//! the window, and the frontend converts client coordinates to physical
//! pixels via the same scale factor the orchestrator computed for the
//! captured framebuffer.
//!
//! `show_over_virtual_desktop` is the single backend seam for the overlay
//! reveal contract (issue #60): it positions the window, shows it, and
//! tells the [`SessionOrchestrator`](crate::session::SessionOrchestrator)
//! the overlay has been mounted. The frontend's `OverlayApp.svelte` only
//! has to render the freeze frame — it no longer needs to know that
//! mounting the overlay implies a state-machine step.

use pixelgrab_contracts::{
    coordinate::PhysicalBounds, monitor::MonitorLayout, PlatformError, PlatformErrorKind,
    PlatformResult,
};
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl, WebviewWindowBuilder,
};

use crate::session::SessionOrchestrator;

/// Pre-allocate the overlay window and hide it. Subsequent captures reuse
/// the same window and only toggle visibility.
pub fn preallocate<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if app.get_webview_window("overlay").is_some() {
        return Ok(());
    }
    let _ = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("overlay.html".into()))
        .title("PixelGrab Overlay")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .resizable(false)
        .build()?;
    Ok(())
}

/// Position the overlay window over the virtual desktop's bounding
/// rectangle. The overlay window is sized to cover the entire virtual
/// desktop so the user can drag a region that crosses monitor
/// boundaries; the Konva stage's coordinate system is the same physical
/// coordinates the captured framebuffer uses (see ADR-0003).
///
/// The window is positioned and sized explicitly in physical pixels. This
/// preserves negative virtual origins and avoids applying one monitor's scale
/// factor to a mixed-DPI desktop. Returns `Ok` when the overlay is still
/// pre-allocated; its geometry is set lazily on the first reveal.
pub fn position_over_virtual_desktop<R: Runtime>(
    app: &AppHandle<R>,
    layout: &MonitorLayout,
) -> PlatformResult<()> {
    let virtual_bounds = layout.virtual_bounds().ok_or_else(|| {
        PlatformError::new(
            PlatformErrorKind::MonitorQueryFailed,
            "no monitors in layout",
        )
    })?;
    let composite = virtual_bounds.as_top_left_bounds();
    position_over_bounds(app, &composite)
}

/// Position the overlay window over the given physical-pixel rectangle.
/// Public so the platform adapters can target a single monitor
/// (`SingleMonitor` capture) without resizing the WebView beyond what
/// the captured framebuffer can cover.
pub fn position_over_bounds<R: Runtime>(
    app: &AppHandle<R>,
    bounds: &PhysicalBounds,
) -> PlatformResult<()> {
    let window = app.get_webview_window("overlay").ok_or_else(|| {
        PlatformError::new(
            PlatformErrorKind::InvalidSessionState,
            "overlay window is not pre-allocated",
        )
    })?;
    window
        .set_position(PhysicalPosition::new(bounds.origin.x, bounds.origin.y))
        .map_err(overlay_err)?;
    window
        .set_size(PhysicalSize::new(bounds.size.width, bounds.size.height))
        .map_err(overlay_err)?;
    Ok(())
}

/// Show the overlay window over the virtual desktop. **Single backend
/// seam for the overlay reveal contract** (issue #60): positions the
/// window, makes it visible, and walks the orchestrator from `Ready` to
/// `Selecting` via [`SessionOrchestrator::overlay_mounted`]. The
/// frontend's `OverlayApp.svelte` no longer has to drive any of those
/// steps — it just renders the freeze frame the orchestrator already
/// has.
pub fn show_over_virtual_desktop<R: Runtime>(
    app: &AppHandle<R>,
    layout: &MonitorLayout,
    session: &SessionOrchestrator,
) -> PlatformResult<()> {
    position_over_virtual_desktop(app, layout)?;
    show_positioned(app, session)
}

/// Show an overlay positioned over a captured physical rectangle. This is
/// the fallback seam for a transient monitor-layout query failure: a capture
/// already has trustworthy bounds, so it must still become interactive.
pub fn show_over_bounds<R: Runtime>(
    app: &AppHandle<R>,
    bounds: &PhysicalBounds,
    session: &SessionOrchestrator,
) -> PlatformResult<()> {
    position_over_bounds(app, bounds)?;
    show_positioned(app, session)
}

fn show_positioned<R: Runtime>(
    app: &AppHandle<R>,
    session: &SessionOrchestrator,
) -> PlatformResult<()> {
    let window = app.get_webview_window("overlay").ok_or_else(|| {
        PlatformError::new(
            PlatformErrorKind::InvalidSessionState,
            "overlay window is not pre-allocated",
        )
    })?;
    window.show().map_err(overlay_err)?;
    // The overlay window is already on screen — a state-machine
    // rejection here would mean an in-flight cancel raced the
    // transition, which is observable in logs but not fatal. The
    // overlay stays visible; the orchestrator just stays in its
    // current state.
    if let Err(err) = session.overlay_mounted() {
        log::warn!("overlay_mounted rejected after show: {err}");
    }
    Ok(())
}

/// Hide the overlay window. Used by the session cleanup path.
pub fn hide<R: Runtime>(app: &AppHandle<R>) -> PlatformResult<()> {
    let window = app.get_webview_window("overlay").ok_or_else(|| {
        PlatformError::new(
            PlatformErrorKind::InvalidSessionState,
            "overlay window is not pre-allocated",
        )
    })?;
    window.hide().map_err(overlay_err)?;
    Ok(())
}

fn overlay_err(err: tauri::Error) -> PlatformError {
    PlatformError::new(
        PlatformErrorKind::Internal,
        format!("overlay window operation failed: {err}"),
    )
}
