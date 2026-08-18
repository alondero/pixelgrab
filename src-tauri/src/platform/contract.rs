//! The platform contract. Every native capability the orchestrator depends on
//! is hidden behind this trait. Real Windows implementations live in
//! `platform::windows` (introduced in tracer-02). The synthetic implementation
//! lives in `platform::synthetic` and is what the tracer-01 build uses.

use pixelgrab_contracts::{
    capture::{CaptureRequest, CaptureResolution},
    coordinate::PhysicalBounds,
    monitor::MonitorLayout,
    PlatformResult,
};
use std::path::PathBuf;

/// Internal capture error. Maps to `PlatformError` at the IPC boundary.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The requested capture format is not supported by this platform.
    #[error("format not supported: {0:?}")]
    UnsupportedFormat(pixelgrab_contracts::capture::CaptureFormat),
    /// The platform could not enumerate monitors.
    #[error("monitor enumeration failed: {0}")]
    MonitorEnumeration(String),
    /// The underlying capture pipeline failed.
    #[error("capture pipeline failed: {0}")]
    Pipeline(String),
    /// The capture output was rejected (e.g. zero-byte frame).
    #[error("invalid capture output: {0}")]
    InvalidOutput(String),
}

impl From<CaptureError> for pixelgrab_contracts::PlatformError {
    fn from(err: CaptureError) -> Self {
        use pixelgrab_contracts::{PlatformError, PlatformErrorKind};
        let kind = match &err {
            CaptureError::UnsupportedFormat(_) => PlatformErrorKind::Unsupported,
            CaptureError::MonitorEnumeration(_) => PlatformErrorKind::MonitorQueryFailed,
            CaptureError::Pipeline(_) => PlatformErrorKind::CaptureUnavailable,
            CaptureError::InvalidOutput(_) => PlatformErrorKind::CaptureUnavailable,
        };
        PlatformError::new(kind, err.to_string())
    }
}

/// The platform contract.
///
/// All methods are synchronously callable; the orchestrator is responsible for
/// dispatching them onto the appropriate runtime. The contract is intentionally
/// `Send + Sync` so the orchestrator can be wrapped in `Arc` and shared across
/// Tauri command handlers.
pub trait PixelGrabPlatform: std::fmt::Debug + Send + Sync + std::any::Any {
    /// Enumerate the current monitor layout.
    fn monitor_layout(&self) -> PlatformResult<MonitorLayout>;

    /// Run a capture pipeline and return a `CaptureResolution`.
    fn capture(&self, request: &CaptureRequest) -> PlatformResult<CaptureResolution>;

    /// Write the flattened PNG bytes for the given selection. Returns the
    /// absolute path the PNG was written to.
    fn write_png(
        &self,
        capture_id: &str,
        bounds: PhysicalBounds,
        rgba: &[u8],
    ) -> PlatformResult<PathBuf>;
}
