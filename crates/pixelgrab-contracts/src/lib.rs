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

pub mod capture;
pub mod coordinate;
pub mod error;
pub mod ipc;
pub mod monitor;
pub mod session;

pub use capture::{CaptureFormat, CaptureRequest, CaptureResolution};
pub use coordinate::{PhysicalBounds, PhysicalPoint, PhysicalSize, VirtualBounds};
pub use error::{PlatformError, PlatformErrorKind, PlatformResult};
pub use ipc::{
    CaptureIntent, CommitOutcome, CommitRequest, CommitResponse, IpcError, IpcResponse,
    OverlaySelection, RequestCaptureIntent, RequestCommitIntent, RequestOverlayIntent,
    SessionSnapshot,
};
pub use monitor::{MonitorDescriptor, MonitorLayout};
pub use session::{SessionState, SessionTransition};
