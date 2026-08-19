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

pub mod cache;
pub mod capture;
pub mod coordinate;
pub mod drag;
pub mod error;
pub mod ipc;
pub mod monitor;
pub mod session;
pub mod shelf_preferences;
pub mod shelf_queue;

pub use cache::{CacheEntry, CacheEntryMetadata, CaptureId, LockOwner, ShelfId, ShelfPosition};
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
    CancelOutcome, CaptureDiagnostics, CaptureIntent, CaptureResponse, CommitOutcome,
    CommitRequest, CommitResponse, DismissCacheEntryRequest, DismissCacheEntryResponse, IpcError,
    IpcResponse, OverlaySelection, RequestCaptureIntent, RequestCommitIntent, RequestOverlayIntent,
    RequestOverlayResult, SessionSnapshot, ShelfPreferencesDto, ShelfSnapshot,
    StartShelfDragIntent, StartShelfDragResult, UpdateCacheMetadataRequest,
    UpdateShelfPreferencesRequest,
};
pub use monitor::{MonitorDescriptor, MonitorLayout};
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
