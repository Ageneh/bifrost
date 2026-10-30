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

/// One changed setting between two configurations.
///
/// Secret values are redacted before this is built, so it is always safe to
/// serialize.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigChange {
    /// Dotted path of the setting, e.g. `z2m.server1.url`.
    pub path: String,
    /// Previous value, `null` when the setting was added.
    pub before: serde_json::Value,
    /// New value, `null` when the setting was removed.
    pub after: serde_json::Value,
    /// The change involves a secret, so `before`/`after` are redacted.
    pub secret: bool,
}

/// Something applying a configuration does while the bridge keeps running.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ApplyAction {
    /// The setting is read on use and takes effect immediately.
    Update { path: String },
    /// Restart one of the bridge services (http, https, entertainment, mdns, ssdp).
    RestartService { service: String },
    /// Start a newly configured z2m backend.
    StartBackend { name: String },
    /// Stop and remove a z2m backend that is no longer configured.
    StopBackend { name: String },
    /// Re-create a z2m backend from its changed configuration.
    RestartBackend { name: String },
}

/// Why a setting only takes effect after Bifrost restarts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reason {
    pub path: String,
    pub message: String,
}

/// What applying a configuration change involves.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyPlan {
    pub hot: Vec<ApplyAction>,
    pub restart_required: Vec<Reason>,
}
