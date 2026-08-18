//! Capture request and result shapes.

use serde::{Deserialize, Serialize};

use crate::coordinate::PhysicalBounds;

/// Identifies which capture pipeline should run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureFormat {
    /// Full virtual desktop, composited into one RGBA buffer.
    VirtualDesktop,
    /// Single monitor frame (resolved by the platform contract).
    SingleMonitor,
    /// Explicit physical-pixel region (used by the redaction/edit reopen path).
    PhysicalRegion,
}

/// Request to capture a framebuffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRequest {
    /// Which capture pipeline to run.
    pub format: CaptureFormat,
    /// Optional target monitor id (for `SingleMonitor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_id: Option<String>,
    /// Optional explicit region (for `PhysicalRegion`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<PhysicalBounds>,
}

/// Result of a capture pipeline run. The framebuffer is delivered out-of-band
/// through the local asset protocol (PNG bytes); this DTO only carries the
/// metadata needed by the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResolution {
    /// Format used (matches the request).
    pub format: CaptureFormat,
    /// Physical bounds that produced the framebuffer.
    pub bounds: PhysicalBounds,
    /// Asset URL the WebView can load to retrieve the PNG bytes.
    pub asset_url: String,
    /// Monotonic capture id (uuid v4).
    pub capture_id: String,
    /// Frame timestamp in milliseconds since the Unix epoch.
    pub captured_at_ms: i64,
}
