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
use tokio::sync::broadcast;

/// Identity of a device: the server it was learned from, and its IEEE
/// address. Two servers may expose the same IEEE address.
pub type DeviceKey = (String, String);

/// How many events a slow subscriber may fall behind before it is told it
/// missed some. A screen that lags re-reads the snapshot.
const EVENT_BUFFER: usize = 64;

/// The wire types the admin API and the websocket carry, defined once in
/// `bifrost-api` so the UI's bindings are generated from them.
pub use bifrost_api::inventory::{
    Availability, Capabilities, DeviceInfo, DeviceType, ExposeSummary, GroupRef, InventorySnapshot,
    InventoryUpdate, PermitJoinState, ResourceRef, RoomRef, SensorReading, ServerInfo,
};

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

impl From<InventoryEvent> for InventoryUpdate {
    fn from(event: InventoryEvent) -> Self {
        match event {
            InventoryEvent::Device(device) => Self::Device(device),
            InventoryEvent::Removed { server, ieee } => Self::Removed { server, ieee },
            InventoryEvent::Server(server) => Self::Server(server),
        }
    }
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

    /// Record the last value a sensor property reported.
    ///
    /// One entry per property: a motion sensor that also reports temperature
    /// keeps both, and a repeat of the same property replaces its old value
    /// rather than piling up.
    pub fn set_reading(
        &self,
        server: &str,
        ieee: &str,
        property: &str,
        value: serde_json::Value,
        at: DateTime<Utc>,
    ) {
        let updated = {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let Some(device) = state
                .devices
                .get_mut(&(server.to_string(), ieee.to_string()))
            else {
                return;
            };
            let reading = SensorReading {
                property: property.to_string(),
                value,
                at,
            };
            match device
                .readings
                .iter_mut()
                .find(|entry| entry.property == property)
            {
                Some(known) => *known = reading,
                None => device.readings.push(reading),
            }
            device.last_seen = Some(at);
            let snapshot = device.clone();
            state.updated = at;
            snapshot
        };
        self.publish(InventoryEvent::Device(Box::new(updated)));
    }

    /// Record a device's battery level, in percent.
    pub fn set_battery(&self, server: &str, ieee: &str, battery: u8) {
        let updated = {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let Some(device) = state
                .devices
                .get_mut(&(server.to_string(), ieee.to_string()))
            else {
                return;
            };
            if device.battery == Some(battery) {
                return;
            }
            device.battery = Some(battery);
            let snapshot = device.clone();
            state.updated = Utc::now();
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

        let changed: Vec<DeviceInfo> = {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let mut changed = vec![];
            for entry in state.devices.values_mut() {
                if entry.server == name && entry.availability != Availability::Unknown {
                    entry.availability = Availability::Unknown;
                    changed.push(entry.clone());
                }
            }
            state.updated = Utc::now();
            changed
        };

        for device in changed {
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
    let resources = resource_refs(device, device_type);

    DeviceInfo {
        server: server.to_string(),
        ieee: device.ieee_address.to_string(),
        friendly_name: device.friendly_name.clone(),
        model: device.model_id.clone(),
        vendor: device.manufacturer.clone(),
        description: device.description.clone(),
        device_type,
        supported,
        // the backend sets this once it knows whether the device is served
        imported: false,
        unsupported_reason,
        exposes: expose_summaries(device),
        capabilities: capabilities(device),
        availability: previous.map_or(Availability::Unknown, |prev| prev.availability),
        resources,
        room: previous.and_then(|prev| prev.room.clone()),
        group: previous.and_then(|prev| prev.group.clone()),
        last_seen: previous.and_then(|prev| prev.last_seen),
        battery: previous.and_then(|prev| prev.battery),
        readings: previous.map_or_else(Vec::new, |prev| prev.readings.clone()),
    }
}

/// The class of `device`, whether Bifrost can serve it, and why not.
///
/// The classes are the ones the board agreed on. A class Bifrost cannot
/// serve says why, in the record, rather than being left out of the
/// inventory: an invisible device is the thing this wave exists to stop.
#[must_use]
pub fn classify(device: &z2m::api::Device) -> (DeviceType, bool, Option<String>) {
    let exposes = flatten(device.exposes());
    let has = |property: &str| exposes.iter().any(|exp| names(exp, property));

    if device.expose_light().is_some() {
        return (DeviceType::Light, true, None);
    }

    if has("action") {
        return (DeviceType::Remote, true, None);
    }

    if exposes
        .iter()
        .any(|exp| matches!(exp, z2m::api::Expose::Switch(_)))
    {
        return (
            DeviceType::Plug,
            false,
            Some("Bifrost has no Hue mapping for plugs".to_string()),
        );
    }

    // a sensor that reports motion is a motion sensor, whatever else it also
    // reports
    let sensor = if has("occupancy") || has("motion") {
        Some(DeviceType::Motion)
    } else if has("temperature") {
        Some(DeviceType::Temperature)
    } else if has("illuminance") {
        Some(DeviceType::Illuminance)
    } else if has("contact") {
        Some(DeviceType::Contact)
    } else {
        None
    };
    if let Some(sensor) = sensor {
        return (sensor, true, None);
    }

    if let Some(out_of_scope) = out_of_scope_class(&exposes) {
        return (DeviceType::Other, false, Some(out_of_scope.to_string()));
    }

    (
        DeviceType::Other,
        false,
        Some("Bifrost has no Hue mapping for this device".to_string()),
    )
}

/// The reason for a device class the board ruled out of scope.
fn out_of_scope_class(exposes: &[&z2m::api::Expose]) -> Option<&'static str> {
    for expose in exposes {
        let reason = match expose {
            z2m::api::Expose::Cover(_) => "Covers are out of scope by board decision",
            z2m::api::Expose::Climate(_) => "Climate devices are out of scope by board decision",
            z2m::api::Expose::Fan(_) => "Fans are out of scope by board decision",
            z2m::api::Expose::Lock(_) => "Locks are out of scope by board decision",
            _ => continue,
        };
        return Some(reason);
    }
    None
}

/// What the device can do, as far as its exposes say.
#[must_use]
pub fn capabilities(device: &z2m::api::Device) -> Capabilities {
    let exposes = flatten(device.exposes());
    let has = |property: &str| exposes.iter().any(|exp| names(exp, property));

    Capabilities {
        dimmable: has("brightness"),
        color: has("color_xy") || has("color_hs"),
        color_temperature: has("color_temp"),
        gradient: device.expose_gradient().is_some(),
        effects: has("effect"),
        motion: has("occupancy") || has("motion"),
        temperature: has("temperature"),
        illuminance: has("illuminance"),
        contact: has("contact"),
        action: has("action"),
    }
}

/// Every expose of a device, features included, in the order z2m lists them.
#[must_use]
pub fn expose_summaries(device: &z2m::api::Device) -> Vec<ExposeSummary> {
    flatten(device.exposes())
        .into_iter()
        .filter_map(|expose| {
            let base = expose.base();
            let property = base.property.clone()?;
            Some(ExposeSummary {
                property,
                kind: kind_of(expose).to_string(),
                access: base.access,
                unit: unit_of(expose),
                values: values_of(expose),
            })
        })
        .collect()
}

/// A device's exposes with every nested feature flattened in.
fn flatten(exposes: &[z2m::api::Expose]) -> Vec<&z2m::api::Expose> {
    let mut out = Vec::new();
    let mut pending: Vec<&z2m::api::Expose> = exposes.iter().rev().collect();
    while let Some(expose) = pending.pop() {
        out.push(expose);
        pending.extend(expose.base().features.iter().rev());
    }
    out
}

/// Whether `expose` carries the z2m property `property`.
fn names(expose: &z2m::api::Expose, property: &str) -> bool {
    expose.base().property.as_deref() == Some(property)
}

/// The z2m type name of an expose.
const fn kind_of(expose: &z2m::api::Expose) -> &'static str {
    use z2m::api::Expose;
    match expose {
        Expose::Binary(_) => "binary",
        Expose::Composite(_) => "composite",
        Expose::Enum(_) => "enum",
        Expose::Light(_) => "light",
        Expose::Lock(_) => "lock",
        Expose::Numeric(_) => "numeric",
        Expose::Switch(_) => "switch",
        Expose::List(_) => "list",
        Expose::Text(_) => "text",
        Expose::Cover(_) => "cover",
        Expose::Fan(_) => "fan",
        Expose::Climate(_) => "climate",
    }
}

/// The unit a numeric expose is measured in.
fn unit_of(expose: &z2m::api::Expose) -> Option<String> {
    match expose {
        z2m::api::Expose::Numeric(numeric) => numeric.unit.clone(),
        _ => None,
    }
}

/// The values an enum or binary expose accepts.
fn values_of(expose: &z2m::api::Expose) -> Vec<String> {
    match expose {
        z2m::api::Expose::Enum(enumeration) => {
            enumeration.values.iter().map(display_value).collect()
        }
        z2m::api::Expose::Binary(binary) => {
            vec![
                display_value(&binary.value_on),
                display_value(&binary.value_off),
            ]
        }
        _ => vec![],
    }
}

/// A JSON value as the string the wire carries.
fn display_value(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string)
}

/// The Hue resources a device owns, as far as its address decides them.
fn resource_refs(device: &z2m::api::Device, device_type: DeviceType) -> Vec<ResourceRef> {
    let link = |rtype: hue::api::RType| ResourceRef {
        rid: rtype.deterministic(&device.ieee_address).rid.to_string(),
        rtype: rtype_name(rtype).to_string(),
    };

    let mut refs = vec![link(hue::api::RType::Device)];

    match device_type {
        DeviceType::Light => refs.push(link(hue::api::RType::Light)),
        DeviceType::Motion => refs.push(link(hue::api::RType::Motion)),
        DeviceType::Temperature => refs.push(link(hue::api::RType::Temperature)),
        DeviceType::Illuminance => refs.push(link(hue::api::RType::LightLevel)),
        DeviceType::Contact => refs.push(link(hue::api::RType::Contact)),
        DeviceType::Remote => refs.push(link(hue::api::RType::Button)),
        _ => return refs,
    }

    // every served class carries the two companions a client expects
    refs.push(link(hue::api::RType::DevicePower));
    refs.push(link(hue::api::RType::ZigbeeConnectivity));
    refs
}

/// The wire name of a resource type, which is what a `ResourceRef` carries.
const fn rtype_name(rtype: hue::api::RType) -> &'static str {
    match rtype {
        hue::api::RType::Device => "device",
        hue::api::RType::Light => "light",
        hue::api::RType::Motion => "motion",
        hue::api::RType::Temperature => "temperature",
        hue::api::RType::LightLevel => "light_level",
        hue::api::RType::Contact => "contact",
        hue::api::RType::Button => "button",
        hue::api::RType::DevicePower => "device_power",
        hue::api::RType::ZigbeeConnectivity => "zigbee_connectivity",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A z2m device record with the exposes `exposes` and nothing else.
    fn z2m_device(exposes: &serde_json::Value) -> z2m::api::Device {
        let device = json!({
            "description": null,
            "disabled": false,
            "endpoints": {},
            "friendly_name": "Lamp",
            "ieee_address": "0x0017880100000001",
            "interview_completed": true,
            "interviewing": false,
            "manufacturer": "Signify",
            "model_id": "LCT001",
            "network_address": 1234,
            "type": "Router",
            "definition": {
                "model": "LCT001",
                "vendor": "Signify",
                "description": "Hue white and color",
                "exposes": exposes,
                "options": [],
                "supports_ota": false,
            },
        });
        // parsed from text, not a `Value`: the IEEE address deserializer
        // borrows the string it reads
        serde_json::from_str(&device.to_string()).unwrap()
    }

    fn classified(exposes: &serde_json::Value) -> (DeviceType, bool, Option<String>) {
        classify(&z2m_device(exposes))
    }

    #[test]
    fn a_light_is_the_one_class_bifrost_serves() {
        let (device_type, supported, reason) = classified(&json!([{
            "type": "light",
            "features": [
                {"type": "numeric", "property": "brightness", "access": 7},
                {"type": "numeric", "property": "color_temp", "access": 7},
                {"type": "composite", "property": "color_xy", "access": 7},
            ],
        }]));

        assert_eq!(device_type, DeviceType::Light);
        assert!(supported);
        assert_eq!(reason, None);

        let capabilities = capabilities(&z2m_device(&json!([{
            "type": "light",
            "features": [
                {"type": "numeric", "property": "brightness", "access": 7},
                {"type": "numeric", "property": "color_temp", "access": 7},
                {"type": "composite", "property": "color_xy", "access": 7},
            ],
        }])));
        assert!(capabilities.dimmable);
        assert!(capabilities.color_temperature);
        assert!(capabilities.color);
        assert!(!capabilities.motion);
    }

    #[test]
    fn a_switch_is_a_plug_and_says_it_is_not_served() {
        let (device_type, supported, reason) =
            classified(&json!([{"type": "switch", "property": "state", "access": 7}]));

        assert_eq!(device_type, DeviceType::Plug);
        assert!(!supported);
        assert!(reason.unwrap().contains("plugs"));
    }

    #[test]
    fn every_sensor_class_is_recognised_and_served() {
        let cases = [
            (
                json!([{"type": "binary", "property": "occupancy", "access": 1, "value_on": true, "value_off": false}]),
                DeviceType::Motion,
            ),
            (
                json!([{"type": "numeric", "property": "temperature", "access": 1, "unit": "°C"}]),
                DeviceType::Temperature,
            ),
            (
                json!([{"type": "numeric", "property": "illuminance", "access": 1, "unit": "lx"}]),
                DeviceType::Illuminance,
            ),
            (
                json!([{"type": "binary", "property": "contact", "access": 1, "value_on": false, "value_off": true}]),
                DeviceType::Contact,
            ),
        ];

        for (exposes, expected) in cases {
            let (device_type, supported, reason) = classified(&exposes);
            assert_eq!(device_type, expected);
            assert!(supported);
            assert_eq!(reason, None);
        }
    }

    #[test]
    fn a_remote_is_recognised_by_its_action_property() {
        let (device_type, supported, reason) = classified(&json!([
            {"type": "enum", "property": "action", "access": 1, "values": ["on", "off"]}
        ]));

        assert_eq!(device_type, DeviceType::Remote);
        assert!(supported);
        assert_eq!(reason, None);
    }

    #[test]
    fn an_out_of_scope_class_says_so_instead_of_disappearing() {
        for (exposes, needle) in [
            (json!([{"type": "cover", "property": "state"}]), "Covers"),
            (
                json!([{"type": "climate", "property": "system_mode"}]),
                "Climate",
            ),
            (json!([{"type": "fan", "property": "state"}]), "Fans"),
            (json!([{"type": "lock", "property": "state"}]), "Locks"),
        ] {
            let (device_type, supported, reason) = classified(&exposes);
            assert_eq!(device_type, DeviceType::Other);
            assert!(!supported);
            assert!(reason.unwrap().contains(needle));
        }
    }

    #[test]
    fn the_light_classification_is_what_makes_a_device_supported() {
        // the deliberate flip: the same record, once as a light and once as a
        // switch, must land on opposite sides of `supported`
        let light = z2m_device(&json!([{"type": "light", "features": []}]));
        let switch = z2m_device(&json!([{"type": "switch", "property": "state", "access": 7}]));

        assert!(classify(&light).1);
        assert!(!classify(&switch).1);
    }

    #[test]
    fn exposes_are_summarised_with_their_unit_and_values() {
        let device = z2m_device(&json!([
            {"type": "numeric", "property": "temperature", "access": 1, "unit": "°C"},
            {"type": "enum", "property": "action", "access": 1, "values": ["on", "off"]},
        ]));

        let summaries = expose_summaries(&device);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].property, "temperature");
        assert_eq!(summaries[0].unit.as_deref(), Some("°C"));
        assert_eq!(summaries[1].values, ["on", "off"]);
    }

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
            imported: true,
            unsupported_reason: None,
            exposes: vec![],
            capabilities: Capabilities::default(),
            availability: Availability::Online,
            resources: vec![],
            room: None,
            group: None,
            last_seen: None,
            battery: None,
            readings: vec![],
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
    fn a_sensor_reading_is_kept_per_property_and_replaced_in_place() {
        let inventory = Inventory::new();
        let mut events = inventory.subscribe();
        inventory.upsert(device("a", "0x01", "Hall motion"));

        let first = Utc::now();
        inventory.set_reading("a", "0x01", "occupancy", serde_json::json!(true), first);
        inventory.set_reading("a", "0x01", "illuminance", serde_json::json!(312.0), first);

        let device = inventory.device("a", "0x01").unwrap();
        assert_eq!(device.readings.len(), 2);
        assert_eq!(device.readings[0].property, "occupancy");
        assert_eq!(device.readings[0].value, serde_json::json!(true));
        assert_eq!(device.last_seen, Some(first));

        // the same property again replaces its value instead of appending
        let second = first + chrono::Duration::seconds(5);
        inventory.set_reading("a", "0x01", "occupancy", serde_json::json!(false), second);

        let device = inventory.device("a", "0x01").unwrap();
        assert_eq!(device.readings.len(), 2);
        assert_eq!(device.readings[0].value, serde_json::json!(false));
        assert_eq!(device.readings[0].at, second);

        // every change is published, so the screen redraws without polling
        assert!(events.try_recv().is_ok());
    }

    #[test]
    fn a_battery_level_is_published_only_when_it_changes() {
        let inventory = Inventory::new();
        let mut events = inventory.subscribe();
        inventory.upsert(device("a", "0x01", "Hall motion"));
        let _ = events.try_recv().unwrap();

        inventory.set_battery("a", "0x01", 87);
        assert_eq!(inventory.device("a", "0x01").unwrap().battery, Some(87));
        assert!(events.try_recv().is_ok());

        // the same level again is not a change worth a frame
        inventory.set_battery("a", "0x01", 87);
        assert!(events.try_recv().is_err());
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
