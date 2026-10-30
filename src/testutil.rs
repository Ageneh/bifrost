//! Helpers shared by unit tests.

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
        let path = base.join(format!("bifrost-test-{}", uuid::Uuid::new_v4()));
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

/// A minimal configuration in the shape users write today: no ports, no
/// `bifrost` section, so the loader defaults are exercised.
pub const MINIMAL_CONFIG: &str = "\
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 10.12.0.20
  netmask: 255.255.255.0
  gateway: 10.12.0.1
  timezone: Europe/Copenhagen

z2m:
  server1:
    url: ws://10.0.0.100:8080
";
