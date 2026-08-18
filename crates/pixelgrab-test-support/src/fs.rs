//! Isolated filesystem root for tests. Every captured PNG, shelf entry, and
//! settings file that tests produce goes under a single temp directory that
//! is deleted on drop. CI uploads never contain real desktop data.

use std::path::{Path, PathBuf};

/// A uniquely-named root directory under the system temp dir. All test
/// writes are confined to this root.
#[derive(Debug)]
pub struct IsolatedFilesystem {
    root: PathBuf,
}

impl IsolatedFilesystem {
    /// Create a new isolated root. Calling `cleanup` returns the underlying
    /// path so the test can read it before deletion.
    pub fn new(label: &str) -> std::io::Result<Self> {
        let mut root = std::env::temp_dir();
        let unique = format!(
            "pixelgrab-test-{}-{}-{}",
            label,
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
        root.push(unique);
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Absolute path to the isolated root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Join a path under the root.
    pub fn join(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }

    /// Recursively delete the root. Called automatically on drop.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Drop for IsolatedFilesystem {
    fn drop(&mut self) {
        self.cleanup();
    }
}
