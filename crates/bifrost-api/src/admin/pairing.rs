//! Wire types for the pairing admin API under `/bifrost/bridge`.
//!
//! Bifrost has no physical link button, so the admin screen stands in for
//! one: it opens a window, and the v1 `POST /api` call pairs a client while
//! that window is open. The window is a convenience, not access control —
//! the Hue surface is open by design either way.

use serde::{Deserialize, Serialize};

/// Whether a pairing window is open right now.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinkButtonStatus {
    /// True while the window is open.
    pub pressed: bool,
    /// Whole seconds until the window closes; zero when it is closed.
    pub remaining_seconds: u64,
}

/// Body of `POST /bifrost/bridge/link-button`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinkButtonPress {
    /// Seconds the window stays open. Defaults to 30; above 120 is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u64>,
}

/// One pairing attempt, as the history table shows it.
///
/// The history is a log, not an access list: every client shares one fixed
/// username and one fixed DTLS key, so a row cannot be revoked.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PairingRecord {
    /// The free-form `devicetype` string from the v1 request body. It is
    /// client-supplied and unauthenticated, so the UI renders it as text.
    pub devicetype: String,
    /// When the attempt arrived, RFC 3339 in UTC.
    pub at: String,
    /// Whether the bridge handed out a username.
    pub accepted: bool,
}

/// What the pairing screen reads.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PairingHistory {
    /// Whether `POST /api` needs an open window. Always visible on screen.
    pub mode: PairingMode,
    /// Newest first, at most [`PairingRecord`]s the bridge kept.
    pub records: Vec<PairingRecord>,
}

/// Whether `POST /api` requires an open pairing window.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum PairingMode {
    /// Every client may pair at any time; the window is informational only.
    /// This is what an existing install keeps.
    Compat,
    /// A client may pair only while the window is open, as a real bridge
    /// demands.
    Required,
}

impl PairingMode {
    #[must_use]
    pub const fn from_required(require_link_button: bool) -> Self {
        if require_link_button {
            Self::Required
        } else {
            Self::Compat
        }
    }
}

/// One entry of the identity bundle's pairing list.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdentityPairingEntry {
    pub devicetype: String,
    pub at: String,
    pub accepted: bool,
}

/// The identity a bridge can back up and carry to another machine.
///
/// It holds the certificate — private key included — and the pairing list,
/// and nothing else. The z2m token and the admin password are never in it:
/// they are secrets the operator re-enters, which is what a restore asks
/// for.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdentityBundle {
    /// Format version, so a future restore can refuse what it cannot read.
    pub version: u32,
    pub bridge_id: String,
    /// The certificate file verbatim: the private key, then the certificate.
    pub certificate_pem: String,
    pub paired_clients: Vec<IdentityPairingEntry>,
}

/// The certificate is a private key, so `Debug` must never print it.
impl std::fmt::Debug for IdentityBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityBundle")
            .field("version", &self.version)
            .field("bridge_id", &self.bridge_id)
            .field("certificate_pem", &"<<REDACTED>>")
            .field("paired_clients", &self.paired_clients.len())
            .finish()
    }
}

/// Body of `POST /bifrost/bridge/identity/mac`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdentityMacChange {
    /// The new address, e.g. `00:11:22:33:44:55`.
    pub mac: String,
    /// The current bridge id, typed by the user to confirm the change.
    pub confirm: String,
}

/// What a MAC change did, and what it will cost.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdentityMacChangeReply {
    pub mac: String,
    pub bridge_id: String,
    /// Always true: the identity is only rebuilt at startup.
    pub restart_required: bool,
}

/// Body of `POST /bifrost/bridge/identity/restore`.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdentityRestore {
    pub bundle: IdentityBundle,
    /// Authorises the restore. Never stored, never logged.
    pub admin_password: String,
    /// Written into the z2m servers that have no token of their own.
    /// Never stored in the bundle.
    pub z2m_token: String,
}

impl std::fmt::Debug for IdentityRestore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityRestore")
            .field("bundle", &self.bundle)
            .field("admin_password", &"<<REDACTED>>")
            .field("z2m_token", &"<<REDACTED>>")
            .finish()
    }
}

/// What a restore changed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdentityRestoreReply {
    pub bridge_id: String,
    pub paired_clients: usize,
    /// The z2m servers that took the supplied token.
    pub tokens_written: Vec<String>,
}
