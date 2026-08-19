//! Capture-session orchestration. The session state machine is the source
//! of truth for "is a capture in progress?" and enforces the lifecycle
//! guarantees from the parent spec.

pub mod state;

pub use state::{EscapeAction, SessionOrchestrator};

pub use pixelgrab_contracts::session::SessionState;
