//! What is actually on the zigbee network.
//!
//! Every z2m backend publishes what it learns into one [`Inventory`]: the
//! devices of every configured server, keyed by `(server, ieee)` so two
//! servers exposing the same IEEE address stay apart, their availability,
//! and whether the server itself is still talking to us.
//!
//! The registry is a separate lock from [`crate::resource::Resources`] and
//! is never taken while that one is held. Readers take a snapshot; writers
//! publish an [`InventoryEvent`] on a broadcast channel, which is what the
//! websocket relays and what a screen redraws from.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Identity of a device: the server it was learned from, and its IEEE
/// address. Two servers may expose the same IEEE address.
pub type DeviceKey = (String, String);

/// How many events a slow subscriber may fall behind before it is told it
/// missed some. A screen that lags re-reads the snapshot.
const EVENT_BUFFER: usize = 64;

/// What a device can do, as far as Bifrost can tell from its z2m exposes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// Whether a device is reachable, as z2m last reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Online,
    Offline,
    /// Nothing has been heard about this device since the server last
    /// spoke to us; it is neither known to be up nor known to be down.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposeSummary {
    /// The z2m property, e.g. `brightness` or `occupancy`.
    pub property: String,
    /// The expose type, e.g. `numeric` or `binary`.
    pub kind: String,
    /// Whether the property is readable, writable or both.
    pub access: u8,
    /// The unit, where z2m names one.
    pub unit: Option<String>,
    /// The accepted values of an enum or binary expose.
    pub values: Vec<String>,
}

/// A Hue resource this device owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub rid: String,
    pub rtype: String,
}

/// The room a device is in, when it is in one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomRef {
    pub rid: String,
    pub name: String,
}

/// The z2m group a device belongs to, when it belongs to one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRef {
    pub id: u32,
    pub name: String,
}

/// Everything Bifrost knows about one device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// The z2m server the device was learned from.
    pub server: String,
    /// The IEEE address, as z2m prints it.
    pub ieee: String,
    /// The z2m friendly name, which is the topic prefix.
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
}

impl DeviceInfo {
    /// The key this device is stored under.
    #[must_use]
    pub fn key(&self) -> DeviceKey {
        (self.server.clone(), self.ieee.clone())
    }
}

/// The join window a server has open, as z2m last reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        if !self.is_open() {
            return 0;
        }
        let elapsed = now.signed_duration_since(self.opened_at.unwrap_or(now));
        let elapsed = u32::try_from(elapsed.num_seconds().max(0)).unwrap_or(u32::MAX);
        self.time.saturating_sub(elapsed)
    }
}

/// The state of one z2m server, as far as the inventory knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Everything in the inventory, as one value a reader can copy out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventorySnapshot {
    pub servers: Vec<ServerInfo>,
    pub devices: Vec<DeviceInfo>,
    /// When any part of this changed last.
    pub updated: DateTime<Utc>,
}

/// A change to the inventory, published to every subscriber.
#[derive(Debug, Clone)]
pub enum InventoryEvent {
    /// A device was learned, or changed.
    Device(Box<DeviceInfo>),
    /// A device left the network.
    Removed { server: String, ieee: String },
    /// A server connected, dropped, or reported a join window.
    Server(Box<ServerInfo>),
}

#[derive(Debug)]
struct InventoryState {
    devices: BTreeMap<DeviceKey, DeviceInfo>,
    servers: BTreeMap<String, ServerInfo>,
    updated: DateTime<Utc>,
}

/// The registry every z2m backend publishes into.
#[derive(Debug)]
pub struct Inventory {
    state: RwLock<InventoryState>,
    events: broadcast::Sender<InventoryEvent>,
}

impl Inventory {
    /// An empty inventory.
    #[must_use]
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Arc::new(Self {
            state: RwLock::new(InventoryState {
                devices: BTreeMap::new(),
                servers: BTreeMap::new(),
                updated: Utc::now(),
            }),
            events,
        })
    }

    /// Follow every change.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<InventoryEvent> {
        self.events.subscribe()
    }

    /// Copy out the whole inventory.
    #[must_use]
    pub fn snapshot(&self) -> InventorySnapshot {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        InventorySnapshot {
            servers: state.servers.values().cloned().collect(),
            devices: state.devices.values().cloned().collect(),
            updated: state.updated,
        }
    }

    /// One device by key.
    #[must_use]
    pub fn device(&self, server: &str, ieee: &str) -> Option<DeviceInfo> {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        state
            .devices
            .get(&(server.to_string(), ieee.to_string()))
            .cloned()
    }

    /// Every device that has this IEEE address, on any server.
    #[must_use]
    pub fn devices_by_ieee(&self, ieee: &str) -> Vec<DeviceInfo> {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        state
            .devices
            .values()
            .filter(|device| device.ieee == ieee)
            .cloned()
            .collect()
    }

    /// A server by name.
    #[must_use]
    pub fn server(&self, name: &str) -> Option<ServerInfo> {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        state.servers.get(name).cloned()
    }

    /// Learn or replace a device.
    pub fn upsert(&self, device: DeviceInfo) {
        let key = device.key();
        {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            state.updated = Utc::now();
            state.devices.insert(key, device.clone());
        }
        self.publish(InventoryEvent::Device(Box::new(device)));
    }

    /// Forget a device. Returns whether there was one.
    pub fn remove(&self, server: &str, ieee: &str) -> bool {
        let removed = {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let removed = state
                .devices
                .remove(&(server.to_string(), ieee.to_string()))
                .is_some();
            if removed {
                state.updated = Utc::now();
            }
            removed
        };

        if removed {
            self.publish(InventoryEvent::Removed {
                server: server.to_string(),
                ieee: ieee.to_string(),
            });
        }
        removed
    }

    /// Set a device's availability and the time it was last heard from.
    pub fn set_availability(
        &self,
        server: &str,
        ieee: &str,
        availability: Availability,
        last_seen: Option<DateTime<Utc>>,
    ) {
        let updated = {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let Some(device) = state
                .devices
                .get_mut(&(server.to_string(), ieee.to_string()))
            else {
                return;
            };
            device.availability = availability;
            if last_seen.is_some() {
                device.last_seen = last_seen;
            }
            let snapshot = device.clone();
            state.updated = Utc::now();
            snapshot
        };
        self.publish(InventoryEvent::Device(Box::new(updated)));
    }

    /// Record that a device reported something, without changing anything
    /// else about it.
    pub fn touch(&self, server: &str, ieee: &str, at: DateTime<Utc>) {
        let updated = {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let Some(device) = state
                .devices
                .get_mut(&(server.to_string(), ieee.to_string()))
            else {
                return;
            };
            device.last_seen = Some(at);
            let snapshot = device.clone();
            state.updated = at;
            snapshot
        };
        self.publish(InventoryEvent::Device(Box::new(updated)));
    }

    /// Replace a device's room and group refs.
    pub fn set_placement(
        &self,
        server: &str,
        ieee: &str,
        room: Option<RoomRef>,
        group: Option<GroupRef>,
    ) {
        let updated = {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let Some(device) = state
                .devices
                .get_mut(&(server.to_string(), ieee.to_string()))
            else {
                return;
            };
            if device.room == room && device.group == group {
                return;
            }
            device.room = room;
            device.group = group;
            let snapshot = device.clone();
            state.updated = Utc::now();
            snapshot
        };
        self.publish(InventoryEvent::Device(Box::new(updated)));
    }

    /// Record a server's state.
    pub fn set_server(&self, server: ServerInfo) {
        {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            state.updated = Utc::now();
            state.servers.insert(server.name.clone(), server.clone());
        }
        self.publish(InventoryEvent::Server(Box::new(server)));
    }

    /// A server connected.
    pub fn server_up(&self, name: &str) {
        let mut server = self.server(name).unwrap_or_else(|| ServerInfo::new(name));
        server.online = true;
        server.last_seen = Some(Utc::now());
        self.set_server(server);
    }

    /// A server stopped talking to us: its devices are no longer known to be
    /// reachable, but they are not forgotten either.
    pub fn server_down(&self, name: &str) {
        let mut server = self.server(name).unwrap_or_else(|| ServerInfo::new(name));
        server.online = false;
        self.set_server(server);

        let stale: Vec<DeviceInfo> = {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let mut stale = vec![];
            for device in state.devices.values_mut() {
                if device.server == name && device.availability != Availability::Unknown {
                    device.availability = Availability::Unknown;
                    stale.push(device.clone());
                }
            }
            state.updated = Utc::now();
            stale
        };

        for device in stale {
            self.publish(InventoryEvent::Device(Box::new(device)));
        }
    }

    /// Record a join window on a server.
    pub fn set_permit_join(&self, name: &str, permit_join: PermitJoinState) {
        let mut server = self.server(name).unwrap_or_else(|| ServerInfo::new(name));
        server.permit_join = permit_join;
        server.last_seen = Some(Utc::now());
        self.set_server(server);
    }

    /// Forget a server that is no longer configured.
    pub fn remove_server(&self, name: &str) {
        let mut removed = Vec::new();
        {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            state.servers.remove(name);

            let keys: Vec<DeviceKey> = state
                .devices
                .keys()
                .filter(|(owner, _)| *owner == name)
                .cloned()
                .collect();
            for key in keys {
                state.devices.remove(&key);
                removed.push(key.1);
            }
            drop(state);
        }

        for ieee in removed {
            self.publish(InventoryEvent::Removed {
                server: name.to_string(),
                ieee,
            });
        }
    }

    fn publish(&self, event: InventoryEvent) {
        // a send error only means nobody is listening
        let _ = self.events.send(event);
    }
}

/// Classify a z2m device and turn it into the registry's record.
///
/// `availability` is carried over from the previous record, because a
/// re-publication of `bridge/devices` says nothing about reachability.
#[must_use]
pub fn from_z2m(
    server: &str,
    device: &z2m::api::Device,
    previous: Option<&DeviceInfo>,
) -> DeviceInfo {
    let (device_type, supported, unsupported_reason) = classify(device);
    let resources = resource_refs(device, supported);

    DeviceInfo {
        server: server.to_string(),
        ieee: device.ieee_address.to_string(),
        friendly_name: device.friendly_name.clone(),
        model: device.model_id.clone(),
        vendor: device.manufacturer.clone(),
        description: device.description.clone(),
        device_type,
        supported,
        unsupported_reason,
        exposes: vec![],
        capabilities: Capabilities::default(),
        availability: previous.map_or(Availability::Unknown, |prev| prev.availability),
        resources,
        room: previous.and_then(|prev| prev.room.clone()),
        group: previous.and_then(|prev| prev.group.clone()),
        last_seen: previous.and_then(|prev| prev.last_seen),
    }
}

/// The class of `device`, whether Bifrost can serve it, and why not.
#[must_use]
pub fn classify(device: &z2m::api::Device) -> (DeviceType, bool, Option<String>) {
    if device.expose_light().is_some() {
        return (DeviceType::Light, true, None);
    }

    (
        DeviceType::Other,
        false,
        Some("Bifrost has no Hue mapping for this device".to_string()),
    )
}

/// The Hue resources a device owns, as far as its address decides them.
fn resource_refs(device: &z2m::api::Device, supported: bool) -> Vec<ResourceRef> {
    let device_ref = ResourceRef {
        rid: hue::api::RType::Device
            .deterministic(&device.ieee_address)
            .rid
            .to_string(),
        rtype: "device".to_string(),
    };

    if !supported {
        return vec![device_ref];
    }

    vec![
        device_ref,
        ResourceRef {
            rid: hue::api::RType::Light
                .deterministic(&device.ieee_address)
                .rid
                .to_string(),
            rtype: "light".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(server: &str, ieee: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            server: server.to_string(),
            ieee: ieee.to_string(),
            friendly_name: name.to_string(),
            model: None,
            vendor: None,
            description: None,
            device_type: DeviceType::Light,
            supported: true,
            unsupported_reason: None,
            exposes: vec![],
            capabilities: Capabilities::default(),
            availability: Availability::Online,
            resources: vec![],
            room: None,
            group: None,
            last_seen: None,
        }
    }

    #[test]
    fn the_same_ieee_on_two_servers_does_not_collide() {
        let inventory = Inventory::new();
        inventory.upsert(device("a", "0x01", "Lamp A"));
        inventory.upsert(device("b", "0x01", "Lamp B"));

        assert_eq!(inventory.snapshot().devices.len(), 2);
        assert_eq!(
            inventory.device("a", "0x01").unwrap().friendly_name,
            "Lamp A"
        );
        assert_eq!(
            inventory.device("b", "0x01").unwrap().friendly_name,
            "Lamp B"
        );
        assert_eq!(inventory.devices_by_ieee("0x01").len(), 2);
    }

    #[test]
    fn removing_a_device_publishes_and_forgets_it() {
        let inventory = Inventory::new();
        let mut events = inventory.subscribe();
        inventory.upsert(device("a", "0x01", "Lamp"));

        // drain the upsert
        let _ = events.try_recv().unwrap();

        assert!(inventory.remove("a", "0x01"));
        assert!(matches!(
            events.try_recv().unwrap(),
            InventoryEvent::Removed { .. }
        ));
        assert!(inventory.device("a", "0x01").is_none());
        assert!(!inventory.remove("a", "0x01"));
    }

    #[test]
    fn a_server_going_down_makes_its_devices_unknown() {
        let inventory = Inventory::new();
        inventory.upsert(device("a", "0x01", "Lamp"));
        inventory.upsert(device("b", "0x01", "Lamp B"));

        inventory.server_down("a");

        assert_eq!(
            inventory.device("a", "0x01").unwrap().availability,
            Availability::Unknown
        );
        // another server's devices are untouched
        assert_eq!(
            inventory.device("b", "0x01").unwrap().availability,
            Availability::Online
        );
        assert!(!inventory.server("a").unwrap().online);
    }

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
        assert_eq!(
            state.remaining_seconds(opened + chrono::Duration::seconds(20)),
            40
        );
        // a window cannot count below zero
        assert_eq!(
            state.remaining_seconds(opened + chrono::Duration::seconds(120)),
            0
        );

        let closed = PermitJoinState {
            time: 0,
            device: None,
            opened_at: None,
        };
        assert!(!closed.is_open());
        assert_eq!(closed.remaining_seconds(opened), 0);
    }
}
