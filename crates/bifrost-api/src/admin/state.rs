//! Wire types for the bridge's own state under `/bifrost/state`.
//!
//! The state holds what pairing and use have built up: the Hue users, the
//! scenes, the resources the z2m backends imported. The configuration holds
//! the settings. Resetting the state never touches the configuration.

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::error::BifrostResult;

/// What the bridge's state holds, and what it is.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StateSummary {
    /// The bridge id, derived from the MAC address.
    pub bridge_id: String,
    /// The MAC address the bridge id derives from.
    pub mac: String,
    /// The configured bridge name.
    pub name: String,
    /// Firmware version, as the Hue API reports it.
    pub version: String,
    /// How many of each resource the state holds.
    pub counts: StateCounts,
}

/// How many of each resource the state holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StateCounts {
    pub lights: usize,
    pub grouped_lights: usize,
    pub rooms: usize,
    pub devices: usize,
    pub scenes: usize,
    pub entertainment_configurations: usize,
}

/// Reset the state. The typed bridge id is the confirmation: a request that
/// does not carry it is refused, so a stray click cannot empty the bridge.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StateResetRequest {
    /// The bridge id from [`StateSummary`], typed by the user.
    pub confirm: String,
}

impl Client {
    /// Counts, bridge id and version.
    pub async fn state_summary(&self) -> BifrostResult<StateSummary> {
        self.get("state/summary").await
    }

    /// Reset the state, after the user typed `confirm`.
    pub async fn reset_state(&self, confirm: &str) -> BifrostResult<StateSummary> {
        self.post(
            "state/reset",
            StateResetRequest {
                confirm: confirm.to_string(),
            },
        )
        .await
    }
}
