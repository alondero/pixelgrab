//! Platform-neutral contracts, error types, and IPC payloads for PixelGrab.
//!
//! This crate is the single source of truth for shapes that cross the
//! Rust/IPC/Svelte boundary. Field names and semantics are mirrored in
//! `src/lib/ipc/types.ts` and verified by the contract tests in
//! `src-tauri/tests/ipc_contracts.rs` and `src/lib/ipc/types.test.ts`.
//!
//! See ADR-0002 (platform contracts) and ADR-0003 (physical-coordinate
//! ownership) for the rationale.

#![deny(missing_docs)]
#![deny(rust_2018_idioms)]

pub mod annotation;
pub mod cache;
pub mod capture;
pub mod coordinate;
pub mod drag;
pub mod error;
pub mod ipc;
pub mod monitor;
pub mod pin;
pub mod session;
pub mod shelf_preferences;
pub mod shelf_queue;

pub use annotation::{
    flatten_annotations, paint_annotation, Annotation, AnnotationColor, AnnotationGeometry,
    AnnotationId, AnnotationStroke, BADGE_RADIUS_PX,
};
pub use cache::{
    CacheEntry, CacheEntryMetadata, CachePolicy, CacheStats, CaptureId, LockOwner, ShelfId,
    ShelfPosition, SweepOutcome, CACHE_POLICY_SCHEMA_VERSION, DEFAULT_LOW_WATER_RATIO,
    DEFAULT_MAX_AGE_MS, DEFAULT_MAX_BYTES, DEFAULT_MAX_ENTRIES, DEFAULT_SWEEP_INTERVAL_MS,
    MAX_LOW_WATER_RATIO, MAX_SWEEP_INTERVAL_MS, MIN_LOW_WATER_RATIO, MIN_MAX_AGE_MS, MIN_MAX_BYTES,
    MIN_MAX_ENTRIES, MIN_SWEEP_INTERVAL_MS,
};
pub use capture::{CaptureFormat, CaptureRequest, CaptureResolution};
pub use coordinate::{
    transform, ClientBounds, ClientPoint, ClientSize, PhysicalBounds, PhysicalPoint, PhysicalSize,
    VirtualBounds,
};
pub use drag::{
    DragDiagnostics, DragFormat, DragFormatRequest, DragOutcome, DragRequest, DragResult,
    DragTargetEffect, DragTargetKind,
};
pub use error::{PlatformError, PlatformErrorKind, PlatformResult};
pub use ipc::{
    CachePolicyDto, CacheStatsResponse, CancelOutcome, CaptureDiagnostics, CaptureIntent,
    CaptureResponse, ClearCacheResponse, CommitOutcome, CommitRequest, CommitResponse,
    DismissCacheEntryRequest, DismissCacheEntryResponse, IpcError, IpcResponse, OverlaySelection,
    RequestCaptureIntent, RequestCommitIntent, RequestOverlayIntent, RequestOverlayResult,
    SaveCaptureAsRequest, SaveCaptureAsResponse, SessionSnapshot, ShelfPreferencesDto,
    ShelfSnapshot, StartShelfDragIntent, StartShelfDragResult, UpdateCacheMetadataRequest,
    UpdateCachePolicyRequest, UpdateShelfPreferencesRequest,
};
pub use monitor::{MonitorDescriptor, MonitorLayout};
pub use pin::{
    clamp_opacity, clamp_zoom, cursor_centered_zoom, limits as pin_limits, reanchor, scaled,
    OpenPinRequest, PinAction, PinActionOutcome, PinCommand, PinId, PinLifecycle, PinLockProvider,
    PinSource, PinTransform, PinViewModel,
};
pub use session::{SessionState, SessionTransition};
pub use shelf_preferences::{
    placement_for, ShelfCorner, ShelfPreferences, ShelfTimerConfigLike, MAX_LIFETIME_SECONDS,
    MAX_MARGIN_PX, MIN_LIFETIME_SECONDS, MIN_MARGIN_PX, MIN_VISIBLE_CARDS, SETTINGS_SCHEMA_VERSION,
};
pub use shelf_queue::{
    CopyShelfCardRequest, CopyShelfCardResponse, SaveShelfCardAsRequest, SaveShelfCardAsResponse,
    ShelfQueueCard, ShelfQueueSnapshot, ShelfTimerConfig, ShelfTimerState,
    DEFAULT_CARD_LIFETIME_MS, DEFAULT_HOVER_GRACE_MS, MAX_VISIBLE_CARDS,
};
