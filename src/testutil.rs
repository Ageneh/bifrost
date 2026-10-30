//! Helpers shared by unit tests.

use std::fs;
use std::sync::{Arc, Mutex, Once};

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, TimeDelta, Utc};

use crate::auth::{Clock, HashCost};

/// scrypt at a cost low enough for tests; never for real passwords.
pub const FAST_HASH: HashCost = HashCost {
    log_n: 4,
    r: 8,
    p: 1,
};

/// A clock that only moves when told to.
pub struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Utc::now()),
        })
    }

    pub fn advance(&self, by: TimeDelta) {
        *self.now.lock().unwrap() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }
}

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

static LOG_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LOGGER: CapturingLogger = CapturingLogger;
static INSTALL_LOGGER: Once = Once::new();

/// Records every log line of the test process, at every level.
struct CapturingLogger;

impl log::Log for CapturingLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let line = format!("{} {}: {}", record.level(), record.target(), record.args());
        LOG_LINES.lock().unwrap().push(line);
    }

    fn flush(&self) {}
}

/// Start capturing log lines. Tests run in parallel in one process, so the
/// capture holds every test's lines; look for values unique to your test.
pub fn capture_logs() {
    INSTALL_LOGGER.call_once(|| {
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// Every line captured since [`capture_logs`] was first called.
#[must_use]
pub fn captured_logs() -> String {
    LOG_LINES.lock().unwrap().join("\n")
}
