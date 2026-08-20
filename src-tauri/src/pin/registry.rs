//! Pin registry. Owns the per-pin state, the lock guard, and the pure
//! view-model math. Every command is a pure function on the registry's
//! state so the test suite can exercise the full lifecycle without a
//! Tauri runtime.
//!
//! The registry is the single owner of:
//!
//! - the `PinId` allocator (UUIDv4),
//! - the per-pin `PinViewModel` (transform + source),
//! - the per-pin `PinLockGuard` (RAII cache lock),
//! - the display-change hook that re-anchors orphan pins.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use pixelgrab_contracts::{
    clamp_opacity, clamp_zoom, cursor_centered_zoom, reanchor, scaled, OpenPinRequest, PinCommand,
    PinId, PinLockProvider, PinSource, PinTransform, PinViewModel, PlatformError,
    PlatformErrorKind, PlatformResult,
};
use uuid::Uuid;

use super::lock::PinLockGuard;

/// Maximum pin count per process. The shelf is also bounded by the cache
/// LRU; this is a hard ceiling so a runaway open path cannot exhaust
/// resources.
pub const MAX_PINS: usize = 32;

/// Stagger between successive pin positions when the caller does not
/// supply one. The cascade prevents pins from stacking on the primary
/// monitor's origin when the user re-pins several captures in a row.
const PIN_STAGGER_PX: i32 = 24;

/// One pin's registry state. The view model is recomputed on every
/// command; the lock guard is dropped exactly once when the pin closes.
pub struct PinEntry {
    /// Per-pin view model.
    pub view: PinViewModel,
    /// Cache lock held while the pin is open.
    pub lock: PinLockGuard,
}

/// The pin registry. Shared across the IPC layer as `Arc<PinRegistry>`.
pub struct PinRegistry {
    inner: Mutex<PinRegistryInner>,
    lock_provider: Arc<dyn PinLockProvider>,
}

struct PinRegistryInner {
    pins: HashMap<PinId, PinEntry>,
}

impl PinRegistry {
    /// Build a new registry backed by the given lock provider.
    pub fn new(lock_provider: Arc<dyn PinLockProvider>) -> Self {
        Self {
            inner: Mutex::new(PinRegistryInner {
                pins: HashMap::new(),
            }),
            lock_provider,
        }
    }

    /// Open a new pin. Returns the new pin's view model.
    pub fn open(&self, request: OpenPinRequest) -> PlatformResult<PinViewModel> {
        if request.png_path.is_empty() {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                "pin open: png_path must not be empty",
            ));
        }
        let source_size = request.bounds.size;
        if source_size.width == 0 || source_size.height == 0 {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidPayload,
                "pin open: source size must be non-zero",
            ));
        }
        let position = match request.initial_position {
            Some(pos) => pos,
            None => self.staggered_position(),
        };
        let source = PinSource {
            capture_id: request.capture_id.clone(),
            png_path: Some(request.png_path),
            bounds: request.bounds,
        };
        let transform = PinTransform::at(position, source_size);
        let id = PinId(Uuid::new_v4().to_string());

        // Capacity check before we acquire the cache lock — the lock is
        // an external resource and we don't want to charge a ref when we
        // have no slot to put the pin in.
        {
            let inner = self.inner.lock();
            if inner.pins.len() >= MAX_PINS {
                return Err(PlatformError::new(
                    PlatformErrorKind::Internal,
                    format!("pin capacity {} reached", MAX_PINS),
                ));
            }
        }

        // Acquire the cache lock. A double-open (re-pin of the same
        // capture) reports `false` here — the provider's reference count
        // holds the lock once for both pins. The guard is the sole
        // release path: each pin closes its own ref.
        self.lock_provider.acquire(&source.capture_id);
        let lock = PinLockGuard::new(self.lock_provider.clone(), source.capture_id.clone());

        let view = PinViewModel {
            id: id.clone(),
            transform,
            source,
        };

        let mut inner = self.inner.lock();
        inner.pins.insert(
            id.clone(),
            PinEntry {
                view: view.clone(),
                lock,
            },
        );
        Ok(view)
    }

    /// Close a pin. The lock guard is dropped here, releasing the cache
    /// lock. Any id that does not match an open pin is rejected.
    pub fn close(&self, id: &PinId) -> PlatformResult<()> {
        let mut inner = self.inner.lock();
        let entry = inner.pins.remove(id).ok_or_else(|| {
            PlatformError::new(PlatformErrorKind::InvalidPayload, "pin not found")
        })?;
        drop(entry.lock);
        Ok(())
    }

    /// Apply a command to a pin. Returns the updated view model.
    pub fn apply(&self, id: &PinId, command: PinCommand) -> PlatformResult<PinViewModel> {
        let mut inner = self.inner.lock();
        let entry = inner.pins.get_mut(id).ok_or_else(|| {
            PlatformError::new(PlatformErrorKind::InvalidPayload, "pin not found")
        })?;
        apply_command(&mut entry.view.transform, command);
        Ok(entry.view.clone())
    }

    /// Return the view model for one pin.
    pub fn view(&self, id: &PinId) -> PlatformResult<PinViewModel> {
        let inner = self.inner.lock();
        inner
            .pins
            .get(id)
            .map(|e| e.view.clone())
            .ok_or_else(|| PlatformError::new(PlatformErrorKind::InvalidPayload, "pin not found"))
    }

    /// Return a snapshot of all view models.
    pub fn list(&self) -> Vec<PinViewModel> {
        let inner = self.inner.lock();
        let mut views: Vec<PinViewModel> = inner.pins.values().map(|e| e.view.clone()).collect();
        views.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        views
    }

    /// Re-anchor any pin whose top-left falls outside the supplied work
    /// area. The work area is the union of all reachable monitor work
    /// areas; if the pin is fully outside that union, it is anchored to
    /// the top-left of the work area. Zoom and opacity are unchanged.
    pub fn handle_display_change(&self, work_area: pixelgrab_contracts::PhysicalBounds) {
        let mut inner = self.inner.lock();
        for entry in inner.pins.values_mut() {
            let new_position = reanchor(
                entry.view.transform.position,
                entry.view.transform.window_size,
                work_area,
            );
            entry.view.transform.position = new_position;
        }
    }

    /// Current pin count. Used by tests to verify the registry does not
    /// leak pins across repeat cycles.
    pub fn len(&self) -> usize {
        self.inner.lock().pins.len()
    }

    /// `true` when no pins are open. The companion to `len` is required
    /// by the clippy `len_without_is_empty` lint.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().pins.is_empty()
    }

    /// Compute a staggered top-left position so successive pins do not
    /// stack on the primary monitor's origin. The cascade is a fixed
    /// pixel offset per pin, modulo the work area so the pin never
    /// lands out of bounds.
    fn staggered_position(&self) -> pixelgrab_contracts::PhysicalPoint {
        let inner = self.inner.lock();
        let index = inner.pins.len() as i32;
        let offset = (index + 1) * PIN_STAGGER_PX;
        pixelgrab_contracts::PhysicalPoint::new(offset, offset)
    }
}

/// Apply a command to a transform in place. Pure: no I/O, no side effects.
fn apply_command(transform: &mut PinTransform, command: PinCommand) {
    match command {
        PinCommand::Drag { dx, dy } => {
            let new_x = transform.position.x.saturating_add(dx);
            let new_y = transform.position.y.saturating_add(dy);
            transform.position = pixelgrab_contracts::PhysicalPoint::new(new_x, new_y);
        }
        PinCommand::Zoom {
            factor,
            cursor_x,
            cursor_y,
        } => {
            if !factor.is_finite() || factor <= 0.0 {
                return;
            }
            let cursor = pixelgrab_contracts::PhysicalPoint::new(cursor_x as i32, cursor_y as i32);
            let (new_position, new_zoom) =
                cursor_centered_zoom(transform.position, cursor, factor, transform.zoom);
            transform.zoom = new_zoom;
            transform.position = new_position;
            transform.window_size = scaled(transform.source_size, new_zoom);
        }
        PinCommand::SetOpacity { opacity } => {
            transform.opacity = clamp_opacity(opacity);
        }
        PinCommand::Reset => {
            transform.zoom = clamp_zoom(1.0);
            transform.opacity = clamp_opacity(1.0);
            transform.window_size = scaled(transform.source_size, transform.zoom);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixelgrab_contracts::coordinate::{PhysicalBounds, PhysicalSize};

    fn provider() -> Arc<InMemoryPinLockProviderStub> {
        Arc::new(InMemoryPinLockProviderStub::new())
    }

    struct InMemoryPinLockProviderStub {
        state: Mutex<HashMap<String, usize>>,
    }

    impl InMemoryPinLockProviderStub {
        fn new() -> Self {
            Self {
                state: Mutex::new(HashMap::new()),
            }
        }
    }

    impl std::fmt::Debug for InMemoryPinLockProviderStub {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("InMemoryPinLockProviderStub").finish()
        }
    }

    impl PinLockProvider for InMemoryPinLockProviderStub {
        fn acquire(&self, capture_id: &str) -> bool {
            let mut state = self.state.lock();
            let entry = state.entry(capture_id.to_string()).or_insert(0);
            let was_zero = *entry == 0;
            *entry += 1;
            was_zero
        }
        fn release(&self, capture_id: &str) -> bool {
            let mut state = self.state.lock();
            if let Some(count) = state.get_mut(capture_id) {
                if *count > 0 {
                    *count -= 1;
                    if *count == 0 {
                        state.remove(capture_id);
                    }
                    return true;
                }
            }
            false
        }
        fn active_locks(&self) -> usize {
            self.state.lock().values().filter(|c| **c > 0).count()
        }
    }

    fn request(capture_id: &str) -> OpenPinRequest {
        OpenPinRequest {
            capture_id: capture_id.to_string(),
            png_path: format!("/cache/{capture_id}.png"),
            bounds: PhysicalBounds::from_xywh(0, 0, 200, 100),
            initial_position: Some(pixelgrab_contracts::PhysicalPoint::new(40, 40)),
        }
    }

    #[test]
    fn open_close_round_trips_lock_count() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let view = registry.open(request("c1")).expect("open");
        assert_eq!(registry.len(), 1);
        assert_eq!(provider.active_locks(), 1);
        registry.close(&view.id).expect("close");
        assert_eq!(registry.len(), 0);
        assert_eq!(provider.active_locks(), 0);
    }

    #[test]
    fn multiple_pins_have_independent_state() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let a = registry.open(request("c1")).expect("open a");
        let b = registry.open(request("c2")).expect("open b");
        registry
            .apply(&a.id, PinCommand::SetOpacity { opacity: 0.5 })
            .expect("apply a");
        registry
            .apply(&b.id, PinCommand::SetOpacity { opacity: 0.7 })
            .expect("apply b");
        let va = registry.view(&a.id).expect("view a");
        let vb = registry.view(&b.id).expect("view b");
        assert!((va.transform.opacity - 0.5).abs() < 1e-3);
        assert!((vb.transform.opacity - 0.7).abs() < 1e-3);
        assert_eq!(provider.active_locks(), 2);
    }

    #[test]
    fn zoom_keeps_pixel_under_cursor_invariance() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let view = registry
            .open(OpenPinRequest {
                capture_id: "c".to_string(),
                png_path: "/cache/c.png".to_string(),
                bounds: PhysicalBounds::from_xywh(100, 100, 200, 100),
                initial_position: Some(pixelgrab_contracts::PhysicalPoint::new(100, 100)),
            })
            .expect("open");
        // The cursor is at the centre of the window. The window is at
        // (100, 100) with size 200x100, so the cursor's world position is
        // (200, 150). The image pixel under the cursor before zoom is
        // `(world_x - position) / zoom = 100 / 1.0 = 100`.
        let cursor_x = view.transform.window_size.width as f32 / 2.0;
        let cursor_y = view.transform.window_size.height as f32 / 2.0;
        let world_x: f32 = view.transform.position.x as f32 + cursor_x;
        let world_y: f32 = view.transform.position.y as f32 + cursor_y;
        let image_pixel_before_x =
            (world_x - view.transform.position.x as f32) / view.transform.zoom;
        let image_pixel_before_y =
            (world_y - view.transform.position.y as f32) / view.transform.zoom;
        registry
            .apply(
                &view.id,
                PinCommand::Zoom {
                    factor: 2.0,
                    cursor_x,
                    cursor_y,
                },
            )
            .expect("zoom");
        let after = registry.view(&view.id).expect("view");
        // The cursor's world position is fixed across the zoom — the
        // window moves to keep the image pixel under the cursor the same.
        let image_pixel_after_x =
            (world_x - after.transform.position.x as f32) / after.transform.zoom;
        let image_pixel_after_y =
            (world_y - after.transform.position.y as f32) / after.transform.zoom;
        assert!((image_pixel_before_x - image_pixel_after_x).abs() < 1e-3);
        assert!((image_pixel_before_y - image_pixel_after_y).abs() < 1e-3);
        assert!((after.transform.zoom - 2.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_clamps_to_bounds() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let view = registry.open(request("c")).expect("open");
        registry
            .apply(
                &view.id,
                PinCommand::Zoom {
                    factor: 100.0,
                    cursor_x: 0.0,
                    cursor_y: 0.0,
                },
            )
            .expect("zoom up");
        let after = registry.view(&view.id).expect("view");
        assert!(after.transform.zoom <= pixelgrab_contracts::pin_limits::MAX_ZOOM);
        registry
            .apply(
                &view.id,
                PinCommand::Zoom {
                    factor: 0.001,
                    cursor_x: 0.0,
                    cursor_y: 0.0,
                },
            )
            .expect("zoom down");
        let after = registry.view(&view.id).expect("view");
        assert!(after.transform.zoom >= pixelgrab_contracts::pin_limits::MIN_ZOOM);
    }

    #[test]
    fn opacity_clamps_to_bounds() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let view = registry.open(request("c")).expect("open");
        registry
            .apply(&view.id, PinCommand::SetOpacity { opacity: -1.0 })
            .expect("opacity low");
        let after = registry.view(&view.id).expect("view");
        assert!(after.transform.opacity >= pixelgrab_contracts::pin_limits::MIN_OPACITY);
        registry
            .apply(&view.id, PinCommand::SetOpacity { opacity: 2.0 })
            .expect("opacity high");
        let after = registry.view(&view.id).expect("view");
        assert!(after.transform.opacity <= pixelgrab_contracts::pin_limits::MAX_OPACITY);
    }

    #[test]
    fn reset_restores_defaults() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let view = registry.open(request("c")).expect("open");
        registry
            .apply(&view.id, PinCommand::SetOpacity { opacity: 0.3 })
            .expect("opacity");
        registry.apply(&view.id, PinCommand::Reset).expect("reset");
        let after = registry.view(&view.id).expect("view");
        assert!((after.transform.zoom - 1.0).abs() < 1e-3);
        assert!((after.transform.opacity - 1.0).abs() < 1e-3);
    }

    #[test]
    fn drag_shifts_position() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let view = registry.open(request("c")).expect("open");
        // request() places the pin at (40, 40); a drag of (10, -20) lands
        // it at (50, 20).
        registry
            .apply(&view.id, PinCommand::Drag { dx: 10, dy: -20 })
            .expect("drag");
        let after = registry.view(&view.id).expect("view");
        assert_eq!(after.transform.position.x, 50);
        assert_eq!(after.transform.position.y, 20);
    }

    #[test]
    fn repeated_open_close_cycles_release_locks() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        for i in 0..10 {
            let view = registry
                .open(OpenPinRequest {
                    capture_id: format!("c{i}"),
                    png_path: format!("/cache/c{i}.png"),
                    bounds: PhysicalBounds::from_xywh(0, 0, 100, 100),
                    initial_position: None,
                })
                .expect("open");
            registry.close(&view.id).expect("close");
        }
        assert_eq!(registry.len(), 0);
        assert_eq!(provider.active_locks(), 0);
    }

    #[test]
    fn close_releases_lock_for_double_open() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let a = registry.open(request("c")).expect("open a");
        let b = registry.open(request("c")).expect("open b");
        assert_eq!(provider.active_locks(), 1);
        registry.close(&a.id).expect("close a");
        assert_eq!(provider.active_locks(), 1);
        registry.close(&b.id).expect("close b");
        assert_eq!(provider.active_locks(), 0);
    }

    #[test]
    fn display_change_anchors_orphan_but_keeps_zoom() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let view = registry
            .open(OpenPinRequest {
                capture_id: "c".to_string(),
                png_path: "/cache/c.png".to_string(),
                bounds: PhysicalBounds::from_xywh(0, 0, 200, 100),
                initial_position: Some(pixelgrab_contracts::PhysicalPoint::new(5000, 5000)),
            })
            .expect("open");
        registry
            .apply(&view.id, PinCommand::SetOpacity { opacity: 0.5 })
            .expect("opacity");
        let new_work_area = PhysicalBounds::from_xywh(0, 0, 1920, 1080);
        registry.handle_display_change(new_work_area);
        let after = registry.view(&view.id).expect("view");
        assert!(after.transform.position.x < new_work_area.right());
        assert!(
            (after.transform.opacity - 0.5).abs() < 1e-3,
            "opacity preserved"
        );
    }

    #[test]
    fn close_unknown_id_returns_error() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let err = registry.close(&PinId::new("missing")).unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidPayload);
    }

    #[test]
    fn max_pins_enforced() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        for i in 0..MAX_PINS {
            registry
                .open(OpenPinRequest {
                    capture_id: format!("c{i}"),
                    png_path: format!("/cache/c{i}.png"),
                    bounds: PhysicalBounds::from_xywh(0, 0, 10, 10),
                    initial_position: None,
                })
                .expect("open");
        }
        let err = registry
            .open(OpenPinRequest {
                capture_id: "overflow".to_string(),
                png_path: "/cache/overflow.png".to_string(),
                bounds: PhysicalBounds::from_xywh(0, 0, 10, 10),
                initial_position: None,
            })
            .unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::Internal);
        assert_eq!(provider.active_locks(), MAX_PINS);
    }

    #[test]
    fn open_validates_empty_png_path() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let err = registry
            .open(OpenPinRequest {
                capture_id: "c".to_string(),
                png_path: "".to_string(),
                bounds: PhysicalBounds::from_xywh(0, 0, 10, 10),
                initial_position: None,
            })
            .unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidPayload);
    }

    #[test]
    fn open_validates_source_size() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let err = registry
            .open(OpenPinRequest {
                capture_id: "c".to_string(),
                png_path: "/cache/c.png".to_string(),
                bounds: PhysicalBounds::from_xywh(0, 0, 0, 0),
                initial_position: None,
            })
            .unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidPayload);
    }

    #[test]
    fn window_size_scales_with_zoom() {
        let provider = provider();
        let registry = PinRegistry::new(provider.clone());
        let view = registry.open(request("c")).expect("open");
        let initial_size = view.transform.window_size;
        assert_eq!(initial_size, PhysicalSize::new(200, 100));
        registry
            .apply(
                &view.id,
                PinCommand::Zoom {
                    factor: 2.0,
                    cursor_x: 0.0,
                    cursor_y: 0.0,
                },
            )
            .expect("zoom");
        let after = registry.view(&view.id).expect("view");
        assert_eq!(after.transform.window_size, PhysicalSize::new(400, 200));
    }
}
