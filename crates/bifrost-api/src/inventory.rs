//! The device inventory as the admin API and the websocket carry it.
//!
//! These are wire types. The registry that produces them lives in the
//! `bifrost` crate; nothing here knows about zigbee2mqtt.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a device can do, as far as Bifrost can tell from its exposes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
    pub dimmable: bool,
    pub color: bool,
    pub color_temperature: bool,
    pub gradient: bool,
    pub effects: bool,
    /// The device reports motion.
    pub motion: bool,
    /// The device reports temperature.
    pub temperature: bool,
    /// The device reports illuminance.
    pub illuminance: bool,
    /// The device reports an open/closed contact.
    pub contact: bool,
    /// The device sends button events.
    pub action: bool,
}

/// The class of a device, derived from its exposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Light,
    Plug,
    Motion,
    Temperature,
    Illuminance,
    Contact,
    Remote,
    Other,
}

impl DeviceType {
    /// The wire name, for the `type` filter.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Plug => "plug",
            Self::Motion => "motion",
            Self::Temperature => "temperature",
            Self::Illuminance => "illuminance",
            Self::Contact => "contact",
            Self::Remote => "remote",
            Self::Other => "other",
        }
    }

    /// The type named by `name`, if it is one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "light" => Self::Light,
            "plug" => Self::Plug,
            "motion" => Self::Motion,
            "temperature" => Self::Temperature,
            "illuminance" => Self::Illuminance,
            "contact" => Self::Contact,
            "remote" => Self::Remote,
            "other" => Self::Other,
            _ => return None,
        })
    }

    /// Every class, in the order the screen shows its tabs.
    #[must_use]
    pub const fn all() -> [Self; 8] {
        [
            Self::Light,
            Self::Plug,
            Self::Motion,
            Self::Temperature,
            Self::Illuminance,
            Self::Contact,
            Self::Remote,
            Self::Other,
        ]
    }
}

/// Whether a device is reachable, as z2m last reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Online,
    Offline,
    /// Nothing has been heard about this device since its server last spoke
    /// to us; it is neither known to be up nor known to be down.
    Unknown,
}

impl Availability {
    /// The wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Unknown => "unknown",
        }
    }
}

/// One property a device exposes, for the detail view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ExposeSummary {
    /// The z2m property, e.g. `brightness` or `occupancy`.
    pub property: String,
    /// The expose type, e.g. `numeric` or `binary`.
    pub kind: String,
    /// Whether the property is readable, writable or both, as z2m numbers it.
    pub access: u8,
    /// The unit, where z2m names one.
    pub unit: Option<String>,
    /// The accepted values of an enum or binary expose.
    pub values: Vec<String>,
}

/// A Hue resource a device owns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ResourceRef {
    pub rid: String,
    pub rtype: String,
}

/// The room a device is in, when it is in one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct RoomRef {
    pub rid: String,
    pub name: String,
}

/// The z2m group a device belongs to, when it belongs to one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct GroupRef {
    pub id: u32,
    pub name: String,
}

/// The last value one sensor property reported.
///
/// The value is already in the semantics the property names: `occupancy` and
/// `contact` are true when there is motion and when the contact is open, and
/// `temperature` is degrees Celsius in the unit its expose declares.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SensorReading {
    /// The z2m property: `occupancy`, `temperature`, `illuminance`,
    /// `contact` or `action`.
    pub property: String,
    /// The value, in the unit the property's expose declares.
    pub value: Value,
    /// When the device reported it.
    pub at: DateTime<Utc>,
}

/// Everything Bifrost knows about one device.
///
/// Wave 9 extends this additively; no field here may change shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct DeviceInfo {
    /// The z2m server the device was learned from.
    pub server: String,
    /// The IEEE address, as z2m prints it.
    pub ieee: String,
    /// The z2m friendly name, which is also the topic prefix.
    pub friendly_name: String,
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub device_type: DeviceType,
    /// True only where Bifrost can serve the device over the Hue API.
    pub supported: bool,
    /// Why not, for every unsupported device.
    pub unsupported_reason: Option<String>,
    pub exposes: Vec<ExposeSummary>,
    pub capabilities: Capabilities,
    pub availability: Availability,
    /// The Hue resources this device owns.
    pub resources: Vec<ResourceRef>,
    pub room: Option<RoomRef>,
    pub group: Option<GroupRef>,
    /// When the device last reported anything.
    pub last_seen: Option<DateTime<Utc>>,
    /// The battery level in percent, where the device reports one.
    pub battery: Option<u8>,
    /// The last value each sensor property reported, oldest first.
    pub readings: Vec<SensorReading>,
}

impl DeviceInfo {
    /// The pair a device is identified by: two servers may expose the same
    /// IEEE address.
    #[must_use]
    pub fn key(&self) -> (String, String) {
        (self.server.clone(), self.ieee.clone())
    }
}

/// The join window a server has open, as z2m last reported it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PermitJoinState {
    /// Seconds z2m said the window lasts; zero once it has closed.
    pub time: u32,
    /// The device the window is restricted to, if any.
    pub device: Option<String>,
    /// When the window was opened, if it is open.
    pub opened_at: Option<DateTime<Utc>>,
}

impl PermitJoinState {
    /// Whether a join window is open.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.time > 0 && self.opened_at.is_some()
    }

    /// The seconds left in the window, counting down from `opened_at`.
    #[must_use]
    pub fn remaining_seconds(&self, now: DateTime<Utc>) -> u32 {
        let Some(opened_at) = self.opened_at.filter(|_| self.is_open()) else {
            return 0;
        };
        let elapsed = now.signed_duration_since(opened_at);
        let elapsed = u32::try_from(elapsed.num_seconds().max(0)).unwrap_or(u32::MAX);
        self.time.saturating_sub(elapsed)
    }
}

/// The state of one z2m server, as far as the inventory knows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ServerInfo {
    pub name: String,
    /// False once the backend stopped talking to us, whatever the last
    /// device states were.
    pub online: bool,
    pub permit_join: PermitJoinState,
    pub last_seen: Option<DateTime<Utc>>,
}

impl ServerInfo {
    /// A server that has just been configured and not yet connected.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            online: false,
            permit_join: PermitJoinState {
                time: 0,
                device: None,
                opened_at: None,
            },
            last_seen: None,
        }
    }
}

/// The whole inventory, as one value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct InventorySnapshot {
    pub servers: Vec<ServerInfo>,
    pub devices: Vec<DeviceInfo>,
    /// When any part of this changed last.
    pub updated: DateTime<Utc>,
}

/// A change to the inventory, pushed on the websocket.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InventoryUpdate {
    /// Everything, sent once when a socket opens.
    Snapshot(InventorySnapshot),
    /// One device was learned, or changed.
    Device(Box<DeviceInfo>),
    /// One device left the network.
    Removed { server: String, ieee: String },
    /// One server connected, dropped, or reported a join window.
    Server(Box<ServerInfo>),
}

/// What a screen asks of one device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct IdentifyRequest {
    /// How long the device should flash, in seconds.
    pub seconds: Option<u32>,
}

/// What a screen asks of a server's join window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PermitJoinRequest {
    /// How long the window should stay open, in seconds. Absent means the
    /// bridge's default.
    pub time: Option<u32>,
    /// Restrict the window to one device, by friendly name.
    pub device: Option<String>,
}

/// The answer to a permit-join request, and the state of the window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PermitJoinResponse {
    pub server: String,
    /// Seconds left in the window; zero when none is open.
    pub time: u32,
    pub device: Option<String>,
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn a_permit_join_window_counts_down() {
        let opened = Utc::now();
        let state = PermitJoinState {
            time: 60,
            device: None,
            opened_at: Some(opened),
        };

        assert!(state.is_open());
        assert_eq!(state.remaining_seconds(opened), 60);
        assert_eq!(state.remaining_seconds(opened + Duration::seconds(20)), 40);
        // a window cannot count below zero
        assert_eq!(state.remaining_seconds(opened + Duration::seconds(120)), 0);

        let closed = PermitJoinState {
            time: 0,
            device: None,
            opened_at: None,
        };
        assert!(!closed.is_open());
        assert_eq!(closed.remaining_seconds(opened), 0);
    }

    #[test]
    fn every_device_type_round_trips_through_its_wire_name() {
        for device_type in DeviceType::all() {
            assert_eq!(DeviceType::parse(device_type.as_str()), Some(device_type));
        }
        assert_eq!(DeviceType::parse("toaster"), None);
    }
}
