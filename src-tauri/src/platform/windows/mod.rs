//! Windows platform adapter. Implements the `PixelGrabPlatform` contract
//! against the real Windows desktop using `xcap` (which itself wraps the
//! Windows Graphics Capture API). Only compiled on Windows targets; the
//! synthetic platform remains the default for CI.

pub mod capture;
pub mod drag;
pub mod platform;
pub mod work_area;

pub use platform::WindowsPlatform;
