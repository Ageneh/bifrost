//! Wire types for the log viewer under `/bifrost/logs`.

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::error::BifrostResult;

/// One line of the bridge's own log.
///
/// Secrets never reach this buffer: the setup code is printed to standard
/// output instead of logged, and anything that still looks like a secret is
/// redacted before it is stored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogRecord {
    /// When the line was logged, RFC 3339 in UTC.
    pub timestamp: String,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: String,
    /// Module the line came from, e.g. `bifrost::backend::z2m`.
    pub target: String,
    pub message: String,
}

/// The most recent log lines, oldest first.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogList {
    pub records: Vec<LogRecord>,
}

impl Client {
    /// The most recent `limit` log lines, oldest first.
    pub async fn logs(&self, limit: Option<usize>) -> BifrostResult<LogList> {
        let scope = limit.map_or_else(|| "logs".to_string(), |limit| format!("logs?limit={limit}"));
        self.get(&scope).await
    }
}
