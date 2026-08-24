//! Bounded local capture-asset transport (issue #63).
//!
//! The full-frame freeze frame used to cross the WebView boundary as a
//! base64 `data:` URL inside the IPC response — a 4K virtual desktop
//! produces a multi-megabyte string that the serde layer must copy,
//! the IPC channel must buffer, and the JS side must decode. The
//! transport now writes the encoded PNG once to a local file under the
//! cache root and passes only the absolute path; the webview loads it
//! through the Tauri asset protocol (`convertFileSrc`).
//!
//! The byte bound is the safety rail: a malformed or runaway encoder
//! cannot push an unbounded payload into the filesystem or the IPC.

use std::path::{Path, PathBuf};

use pixelgrab_contracts::{PlatformError, PlatformErrorKind, PlatformResult};

/// Maximum accepted PNG size for a capture asset. Generous: a 32K×32K
/// worst-case framebuffer compresses far below this, so the bound only
/// fires on genuine pathology.
pub const MAX_CAPTURE_ASSET_BYTES: usize = 64 * 1024 * 1024;

/// Subdirectory of the cache root that holds per-capture frame assets.
pub const FRAMES_DIR: &str = "frames";

/// Persist the encoded PNG for a capture and return the asset URL the
/// frontend should load.
///
/// With a cache root the URL is the absolute file path (the frontend
/// converts it via the asset protocol). Without one — synthetic CI
/// paths that never configured a root — the function falls back to an
/// inline data URL so the pipeline still works end-to-end.
pub fn write_capture_asset(
    cache_root: Option<&Path>,
    capture_id: &str,
    png_bytes: &[u8],
) -> PlatformResult<String> {
    if png_bytes.len() > MAX_CAPTURE_ASSET_BYTES {
        return Err(PlatformError::new(
            PlatformErrorKind::InvalidPayload,
            format!(
                "capture asset is {} bytes which exceeds the {} byte transport bound",
                png_bytes.len(),
                MAX_CAPTURE_ASSET_BYTES
            ),
        ));
    }
    match cache_root {
        Some(root) => {
            let dir: PathBuf = root.join(FRAMES_DIR);
            // Privacy (AGENTS.md §9, §17 precedent): the io::Error's
            // Display can embed the absolute path on Windows — keep
            // only categorical kinds.
            std::fs::create_dir_all(&dir).map_err(|_| {
                PlatformError::new(PlatformErrorKind::Io, "frame dir create failed")
            })?;
            let path = dir.join(format!("{capture_id}.png"));
            // Write-then-rename keeps a crashed encode from leaving a
            // half-written frame behind (same pattern as the cache's
            // atomic writes).
            let tmp = dir.join(format!("{capture_id}.png.tmp"));
            std::fs::write(&tmp, png_bytes)
                .map_err(|_| PlatformError::new(PlatformErrorKind::Io, "frame write failed"))?;
            std::fs::rename(&tmp, &path)
                .map_err(|_| PlatformError::new(PlatformErrorKind::Io, "frame publish failed"))?;
            Ok(path.to_string_lossy().to_string())
        }
        None => Ok(format!(
            "data:image/png;base64,{}",
            base64_encode(png_bytes)
        )),
    }
}

/// Minimal RFC 4648 base64 encoder (fallback path only).
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let b = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(ALPHABET[((b >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((b >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((b >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(b & 0x3F) as usize] as char);
        i += 3;
    }
    match input.len() - i {
        1 => {
            let b = (input[i] as u32) << 16;
            out.push(ALPHABET[((b >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
            out.push(ALPHABET[((b >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((b >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixelgrab_test_support::fs::IsolatedFilesystem;

    fn tiny_png() -> Vec<u8> {
        // A valid-enough header sequence; the transport never decodes.
        vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 1, 2, 3]
    }

    #[test]
    fn writes_frame_under_cache_root_and_returns_path() {
        let fs = IsolatedFilesystem::new("asset-transport").expect("fs");
        let url = write_capture_asset(Some(fs.root()), "cap-1", &tiny_png()).expect("write");
        let expected = fs.root().join(FRAMES_DIR).join("cap-1.png");
        assert_eq!(url, expected.to_string_lossy());
        assert!(expected.exists(), "frame file exists on disk");
        assert!(std::fs::read(&expected).is_ok());
    }

    #[test]
    fn without_cache_root_falls_back_to_data_url() {
        let url = write_capture_asset(None, "cap-2", &tiny_png()).expect("write");
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn oversize_payload_is_rejected_before_touching_disk() {
        let fs = IsolatedFilesystem::new("asset-transport-big").expect("fs");
        let oversized = vec![0u8; MAX_CAPTURE_ASSET_BYTES + 1];
        let err = write_capture_asset(Some(fs.root()), "cap-3", &oversized).unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::InvalidPayload);
        // Nothing was written.
        assert!(!fs.root().join(FRAMES_DIR).join("cap-3.png").exists());
    }
}
