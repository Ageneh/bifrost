//! Wire types for the configuration admin API under `/bifrost/config`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The configuration must not be applied.
    Error,
    /// The configuration works, but is probably not what was intended.
    Warning,
}

/// One finding of configuration validation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationError {
    /// Dotted path of the offending setting, e.g. `bridge.http_port`.
    /// Empty when the problem concerns the document as a whole.
    pub path: String,
    pub message: String,
    pub severity: Severity,
}

impl ValidationError {
    pub fn error(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: Severity::Error,
        }
    }

    pub fn warning(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: Severity::Warning,
        }
    }

    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
