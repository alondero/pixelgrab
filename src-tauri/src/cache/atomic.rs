//! Atomic write helper. Every asset the cache publishes is written to
//! a sibling `*.tmp` file, fsync'd, then renamed onto its final path.
//!
//! On any error the helper cleans up the temp file before returning, so
//! a caller never has to think about partial files. The only state the
//! caller observes is: the final path exists with the expected bytes,
//! or the final path does not exist and no temp file is left behind.
//!
//! `write_atomic` is the only path through which the cache writes
//! assets — see the module-level docs in `store.rs` for why.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use pixelgrab_contracts::{PlatformError, PlatformErrorKind};

/// Outcome of an atomic write. Either the final file exists with the
/// expected bytes, or neither the final nor the temp file exists.
#[derive(Debug, PartialEq, Eq)]
pub enum AtomicWriteOutcome {
    /// Wrote `bytes.len()` bytes to `final_path` and successfully
    /// renamed the temp file into place.
    Written {
        /// Final path the bytes were committed to.
        final_path: PathBuf,
        /// Number of bytes written.
        bytes: u64,
    },
    /// The path already existed with identical bytes. Treated as a
    /// success so retried commits are idempotent. The byte count is
    /// returned so the caller can record it in the manifest.
    AlreadyDurable {
        /// Final path that already had the same contents.
        final_path: PathBuf,
        /// Number of bytes already on disk.
        bytes: u64,
    },
}

/// Write `bytes` to `final_path` atomically. The final path is created
/// (parent directories included), fsync'd, and renamed into place.
///
/// If a file already exists at `final_path` with byte-identical
/// contents the function returns `AlreadyDurable` without touching
/// the filesystem. If the existing file differs the function fails
/// with `PlatformErrorKind::Io` so a caller cannot accidentally
/// overwrite an unrelated asset.
pub fn write_atomic(final_path: &Path, bytes: &[u8]) -> Result<AtomicWriteOutcome, PlatformError> {
    if let Some(parent) = final_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                PlatformError::new(
                    PlatformErrorKind::Io,
                    format!("create_dir_all({}): {err}", parent.display()),
                )
            })?;
        }
    }

    if final_path.exists() {
        let existing = fs::read(final_path).map_err(|err| {
            PlatformError::new(
                PlatformErrorKind::Io,
                format!("read existing {}: {err}", final_path.display()),
            )
        })?;
        if existing.as_slice() == bytes {
            return Ok(AtomicWriteOutcome::AlreadyDurable {
                final_path: final_path.to_path_buf(),
                bytes: existing.len() as u64,
            });
        }
        return Err(PlatformError::new(
            PlatformErrorKind::Io,
            format!(
                "refusing to overwrite existing file at {} ({} bytes vs new {} bytes)",
                final_path.display(),
                existing.len(),
                bytes.len()
            ),
        ));
    }

    let tmp_path = tmp_path_for(final_path);
    // If a previous crash left a `*.tmp` behind, remove it before
    // starting a fresh write so the rename target is unambiguous.
    let _ = fs::remove_file(&tmp_path);

    let write_result = (|| -> Result<(), PlatformError> {
        let mut file = File::create(&tmp_path).map_err(|err| {
            PlatformError::new(
                PlatformErrorKind::Io,
                format!("create tmp {}: {err}", tmp_path.display()),
            )
        })?;
        file.write_all(bytes).map_err(|err| {
            PlatformError::new(
                PlatformErrorKind::Io,
                format!("write tmp {}: {err}", tmp_path.display()),
            )
        })?;
        file.sync_all().map_err(|err| {
            PlatformError::new(
                PlatformErrorKind::Io,
                format!("fsync tmp {}: {err}", tmp_path.display()),
            )
        })?;
        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }

    if let Err(err) = fs::rename(&tmp_path, final_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(PlatformError::new(
            PlatformErrorKind::Io,
            format!(
                "rename {} -> {}: {err}",
                tmp_path.display(),
                final_path.display()
            ),
        ));
    }

    Ok(AtomicWriteOutcome::Written {
        final_path: final_path.to_path_buf(),
        bytes: bytes.len() as u64,
    })
}

/// Compute the temp path used during an atomic write. We append `.tmp`
/// to the file name so the temp file sits next to the final file and
/// the rename stays within a single directory (no cross-filesystem
/// move). The implementation avoids `with_extension` because that
/// would replace the original extension (e.g. `foo.png` would become
/// `foo.tmp`, not `foo.png.tmp`).
fn tmp_path_for(final_path: &Path) -> PathBuf {
    let mut name = final_path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    match final_path.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pixelgrab_test_support::fs::IsolatedFilesystem;

    #[test]
    fn write_atomic_creates_parent_dirs() {
        let fs = IsolatedFilesystem::new("atomic-parent").expect("fs");
        let target = fs.join("nested/dir/file.bin");
        let outcome = write_atomic(&target, b"hello").expect("write");
        match outcome {
            AtomicWriteOutcome::Written { bytes, .. } => assert_eq!(bytes, 5),
            AtomicWriteOutcome::AlreadyDurable { .. } => panic!("expected fresh write"),
        }
        assert_eq!(fs::read(&target).expect("read"), b"hello");
    }

    #[test]
    fn write_atomic_is_idempotent_for_identical_bytes() {
        let fs = IsolatedFilesystem::new("atomic-idem").expect("fs");
        let target = fs.join("file.bin");
        write_atomic(&target, b"abc").expect("first write");
        let outcome = write_atomic(&target, b"abc").expect("second write");
        assert!(matches!(
            outcome,
            AtomicWriteOutcome::AlreadyDurable { bytes: 3, .. }
        ));
    }

    #[test]
    fn write_atomic_refuses_to_overwrite_different_bytes() {
        let fs = IsolatedFilesystem::new("atomic-overwrite").expect("fs");
        let target = fs.join("file.bin");
        write_atomic(&target, b"abc").expect("first write");
        let err = write_atomic(&target, b"xyz").unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::Io);
        // Original bytes remain intact.
        assert_eq!(fs::read(&target).expect("read"), b"abc");
    }

    #[test]
    fn write_atomic_leaves_no_temp_on_failure() {
        let fs = IsolatedFilesystem::new("atomic-fail").expect("fs");
        // Create a file at a path that will be a "parent" of our target,
        // so `create_dir_all` fails when it tries to descend.
        let blocker = fs.join("blocker");
        std::fs::write(&blocker, b"i am a file, not a directory").expect("write blocker");
        let target = blocker.join("file.bin");
        let err = write_atomic(&target, b"abc").unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::Io);
        // The temp file must not exist.
        let tmp = tmp_path_for(&target);
        assert!(!tmp.exists(), "temp file should be cleaned up on failure");
        // The blocker file must remain intact.
        assert_eq!(
            std::fs::read(&blocker).expect("read blocker"),
            b"i am a file, not a directory",
        );
    }
}
