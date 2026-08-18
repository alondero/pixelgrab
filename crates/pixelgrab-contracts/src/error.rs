//! Structured platform errors that cross the IPC boundary.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result alias using [`PlatformError`].
pub type PlatformResult<T> = Result<T, PlatformError>;

/// Categorises the failure mode so the UI can branch without pattern matching
/// on user-facing strings. The discriminant is stable across releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformErrorKind {
    /// Capture subsystem could not enumerate or grab the requested frame.
    CaptureUnavailable,
    /// Monitor layout could not be queried (e.g. transient WinRT failure).
    MonitorQueryFailed,
    /// The capture session is in an unexpected state (e.g. double-capture).
    InvalidSessionState,
    /// A coordinate transform overflowed or produced a non-finite value.
    CoordinateTransform,
    /// An I/O bound call failed (disk, named pipe, file mapping).
    Io,
    /// A caller passed a payload that failed schema validation.
    InvalidPayload,
    /// The singleton instance refused a second primary launch.
    SingletonConflict,
    /// A platform contract method is not implemented on the current OS.
    Unsupported,
    /// Catch-all for unexpected internal failures.
    Internal,
}

impl std::fmt::Display for PlatformErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::CaptureUnavailable => "capture_unavailable",
            Self::MonitorQueryFailed => "monitor_query_failed",
            Self::InvalidSessionState => "invalid_session_state",
            Self::CoordinateTransform => "coordinate_transform",
            Self::Io => "io",
            Self::InvalidPayload => "invalid_payload",
            Self::SingletonConflict => "singleton_conflict",
            Self::Unsupported => "unsupported",
            Self::Internal => "internal",
        };
        f.write_str(label)
    }
}

/// The wire shape for failures. `kind` is the categorical discriminator and
/// `message` is the human-readable description. `source` (when present) is
/// only the chained error's discriminant - never the underlying message - to
/// avoid leaking sensitive data through IPC.
#[derive(Debug, Error, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformError {
    /// Stable categorical failure mode.
    pub kind: PlatformErrorKind,
    /// Human-readable description. Never contains captured pixels or paths
    /// outside the application cache root.
    pub message: String,
    /// Optional context fields for diagnostics. Values must be redactable.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub context: indexmap::IndexMap<String, String>,
}

use indexmap::IndexMap;

impl PlatformError {
    /// Build a new error with the given kind and message.
    pub fn new(kind: PlatformErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            context: IndexMap::new(),
        }
    }

    /// Add a context field. Key/value pairs must be safe to log.
    #[must_use]
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.kind, self.message)
    }
}

impl From<std::io::Error> for PlatformError {
    fn from(err: std::io::Error) -> Self {
        Self::new(PlatformErrorKind::Io, err.to_string())
    }
}

impl From<serde_json::Error> for PlatformError {
    fn from(err: serde_json::Error) -> Self {
        Self::new(PlatformErrorKind::InvalidPayload, err.to_string())
    }
}
