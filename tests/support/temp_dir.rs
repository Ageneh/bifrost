//! A scratch directory for integration tests.
//!
//! `bifrost::testutil` is compiled into unit tests only, so integration
//! tests carry their own copy of the one helper they need.

// every test binary compiles its own copy of this module, and not all of
// them use the directory
#![allow(dead_code)]

use std::fs;

use camino::{Utf8Path, Utf8PathBuf};

/// A uniquely named directory under the system temp dir, removed on drop.
pub struct TempDir {
    path: Utf8PathBuf,
}

impl TempDir {
    #[must_use]
    pub fn new() -> Self {
        let base = Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        let path = base.join(format!("bifrost-it-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    /// Write `contents` to `name` inside the directory and return the full path.
    #[must_use]
    pub fn write(&self, name: &str, contents: &str) -> Utf8PathBuf {
        let path = self.path.join(name);
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
