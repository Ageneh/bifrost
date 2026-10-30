//! Wire types for rooms under `/bifrost/rooms`.
//!
//! A room is the z2m group the backend imported. Membership is written
//! through the configuration draft (`rooms.<topic>.devices`), so nothing
//! reaches the mesh before apply; scenes are listed and recalled, never
//! authored.

use serde::{Deserialize, Serialize};

/// Every room the bridge knows.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct RoomList {
    pub rooms: Vec<RoomEntry>,
}

/// One room, its devices, and its scenes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct RoomEntry {
    /// The room resource id.
    pub id: String,
    /// The z2m group topic, which is the key under `rooms:` in the file.
    pub topic: String,
    pub name: String,
    /// The room archetype, as the Hue API spells it.
    pub icon: String,
    pub devices: Vec<RoomDevice>,
    pub scenes: Vec<RoomScene>,
}

/// One device in a room.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct RoomDevice {
    /// The IEEE address, which is how `rooms.<topic>.devices` names it.
    pub ieee: String,
    pub name: String,
    /// False when Bifrost cannot serve the device over the Hue API.
    pub supported: bool,
}

/// One scene in a room.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct RoomScene {
    pub id: String,
    pub name: String,
    /// True while this scene is what the room is showing.
    pub active: bool,
}

/// Recall a scene, optionally over `duration` milliseconds.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SceneRecallRequest {
    pub duration: Option<u32>,
}
