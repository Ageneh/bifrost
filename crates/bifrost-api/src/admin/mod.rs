//! Wire types for the configuration admin API under `/bifrost/config`.

pub mod auth;
pub mod certificate;
pub mod logs;
pub mod pairing;
pub mod rooms;
pub mod state;
pub mod z2m;

pub use certificate::CertificateInfo;
pub use logs::{LogList, LogRecord};
pub use pairing::{
    IdentityBundle, IdentityMacChange, IdentityMacChangeReply, IdentityPairingEntry,
    IdentityRestore, IdentityRestoreReply, LinkButtonPress, LinkButtonStatus, PairingHistory,
    PairingMode, PairingRecord,
};
pub use rooms::{RoomDevice, RoomEntry, RoomList, RoomScene, SceneRecallRequest};
pub use state::{StateCounts, StateResetRequest, StateSummary};
pub use z2m::{Z2mServerEntry, Z2mServerList, Z2mServerWrite};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Severity {
    /// The configuration must not be applied.
    Error,
    /// The configuration works, but is probably not what was intended.
    Warning,
}

/// One finding of configuration validation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Reason {
    pub path: String,
    pub message: String,
}

/// What applying a configuration change involves.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApplyPlan {
    pub hot: Vec<ApplyAction>,
    pub restart_required: Vec<Reason>,
}

/// State of the draft, returned by every draft read and write.
///
/// `config` is redacted: a secret that is set reads as `"<<REDACTED>>"`.
/// Secrets are write-only; on write, an absent, `null` or redacted secret
/// keeps the existing value and `""` clears it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DraftResponse {
    /// Revision the next write must name.
    pub revision: u64,
    /// RFC 3339 time of the last draft write, `None` without a draft.
    pub updated_at: Option<String>,
    /// The draft configuration, `None` without a draft.
    pub config: Option<serde_json::Value>,
    /// Findings for the draft.
    pub validation: Vec<ValidationError>,
}

/// Replace the draft.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DraftWrite {
    pub revision: u64,
    pub config: serde_json::Value,
}

/// Change the draft (or, without one, the active configuration) with a
/// JSON merge patch (RFC 7386).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DraftPatch {
    pub revision: u64,
    pub patch: serde_json::Value,
}

/// Replace the draft with an exported YAML document.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConfigImport {
    pub revision: u64,
    pub yaml: String,
}

/// A request that only names the revision it was based on.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RevisionRequest {
    pub revision: u64,
}

/// Restart Bifrost, applying the draft first when one exists (which then
/// requires `revision`).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RestartRequest {
    pub revision: Option<u64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RevisionResponse {
    pub revision: u64,
}

/// Result of applying the draft.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApplyResponse {
    pub revision: u64,
    pub plan: ApplyPlan,
    pub changes: Vec<ConfigChange>,
    /// Work that the apply could not carry out, named by the setting it
    /// belongs to. An apply that reports nothing here changed everything it
    /// was asked to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<ApplyFailure>,
}

/// One part of an apply that did not reach the mesh.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApplyFailure {
    /// The configuration path, e.g. `rooms.Kitchen`.
    pub path: String,
    /// What went wrong, in the words the screen shows.
    pub reason: String,
}

/// Error body of every configuration admin endpoint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ErrorResponse {
    pub error: String,
    /// Validation findings, when the request was refused because of them.
    pub errors: Vec<ValidationError>,
}

/// Where the first-run wizard is, from `GET /bifrost/setup/wizard`.
///
/// The wizard keeps its own record beside the configuration draft, so a
/// half-finished setup is never mistaken for unapplied edits and discarding
/// a draft cannot throw setup away.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WizardProgress {
    /// The step the wizard is on, e.g. `identity`.
    pub step: String,
    /// What the wizard has collected so far, as a partial configuration.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub values: serde_json::Value,
    /// When the record was last written, RFC 3339.
    pub updated_at: Option<String>,
}

/// Write the wizard's progress.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WizardWrite {
    pub step: String,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub values: serde_json::Value,
}

/// Ask a z2m server what it can see, before the bridge exists.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DiscoveryRequest {
    pub server: crate::config::Z2mServer,
}

/// One device a z2m server reported.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DiscoveredDevice {
    /// The IEEE address, which is how an override names the device.
    pub ieee: String,
    pub friendly_name: String,
    pub model: Option<String>,
    pub vendor: Option<String>,
    /// True where Bifrost can serve the device over the Hue API.
    pub supported: bool,
}

/// One zigbee2mqtt group a server reported; a room is this group.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DiscoveredRoom {
    /// The group's friendly name, which is the key under `rooms:`.
    pub topic: String,
    /// The IEEE addresses of the group's members.
    pub members: Vec<String>,
}

/// What a discovery run found.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DiscoveryResult {
    pub devices: Vec<DiscoveredDevice>,
    pub rooms: Vec<DiscoveredRoom>,
}

/// Why Bifrost is in setup mode, from `GET /bifrost/setup/status`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SetupStatus {
    /// The configuration file Bifrost tried to read.
    pub config_file: String,
    /// Why it could not be used: missing, or the parser's message.
    pub reason: String,
    /// A backup of the last applied configuration exists, and
    /// `POST /bifrost/setup/restore` can load it into the draft.
    pub backup_available: bool,
}
