//! Internal error types used by the Tauri backend. The wire shape is
//! `pixelgrab_contracts::PlatformError`; this module is the internal
//! counterpart that knows about Tauri specifics.

use thiserror::Error;

/// Internal error covering the orchestration layer.
#[derive(Debug, Error)]
pub enum PixelGrabError {
    /// The session was in a state that doesn't permit the requested action.
    #[error("invalid session state: {0}")]
    InvalidSessionState(String),
    /// The platform contract returned an error.
    #[error("platform error: {0}")]
    Platform(#[from] pixelgrab_contracts::PlatformError),
    /// The Tauri runtime returned an error.
    #[error("tauri error: {0}")]
    Tauri(#[from] tauri::Error),
    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A JSON (de)serialisation error.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl From<PixelGrabError> for pixelgrab_contracts::PlatformError {
    fn from(err: PixelGrabError) -> Self {
        use pixelgrab_contracts::{PlatformError, PlatformErrorKind};
        let kind = match &err {
            PixelGrabError::InvalidSessionState(_) => PlatformErrorKind::InvalidSessionState,
            PixelGrabError::Platform(_) => PlatformErrorKind::Internal,
            PixelGrabError::Tauri(_) => PlatformErrorKind::Internal,
            PixelGrabError::Io(_) => PlatformErrorKind::Io,
            PixelGrabError::Serde(_) => PlatformErrorKind::InvalidPayload,
        };
        PlatformError::new(kind, err.to_string())
    }
}

/// Convenience alias for `Result<T, PixelGrabError>`.
pub type AppResult<T> = Result<T, PixelGrabError>;
