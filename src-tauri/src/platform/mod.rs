//! Platform contract layer. The trait surface is the boundary between
//! Windows-specific implementations and the platform-neutral orchestration
//! code. The synthetic adapter lives here too so tests can drive the
//! orchestrator without any OS dependency.

pub mod contract;

#[cfg(any(test, feature = "synthetic"))]
pub mod synthetic;

pub use contract::{CaptureError, PixelGrabPlatform};
