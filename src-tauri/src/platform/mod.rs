//! Platform contract layer. The trait surface is the boundary between
//! Windows-specific implementations and the platform-neutral orchestration
//! code. The synthetic adapter lives here too so tests can drive the
//! orchestrator without any OS dependency.

pub mod contract;
pub mod drag_synthetic;

#[cfg(any(test, feature = "synthetic"))]
pub mod synthetic;

#[cfg(target_os = "windows")]
pub mod windows;

pub use contract::{CaptureError, PixelGrabPlatform};
pub use drag_synthetic::{DragOutcomePlan, SyntheticDragScript, SyntheticDragSource};

#[cfg(target_os = "windows")]
pub use windows::WindowsPlatform;
