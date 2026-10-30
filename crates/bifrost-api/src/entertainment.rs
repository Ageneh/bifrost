//! Entertainment streaming, as the admin API reports it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One running DTLS session, for one area.
///
/// `fps_in` and `fps_out` are measured over the last second of polling, not
/// per frame: the diagnostic screen asks once a second, and a per-frame
/// update would cost more than the stream itself.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StreamStats {
    /// The entertainment configuration this session streams.
    pub area: Uuid,
    pub started: DateTime<Utc>,
    /// Frames the sender delivered in the last second.
    pub fps_in: u32,
    /// Frames that reached zigbee2mqtt in the last second, after the
    /// area's throttle.
    pub fps_out: u32,
    /// Every frame this session has received.
    pub frames: u64,
    /// Frames the throttle dropped, so `frames - drops` is what was sent.
    pub drops: u64,
    /// Why the session ended, while it is still listed.
    pub last_error: Option<String>,
}

/// What this bridge can stream on, and what is set up to stream.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentCapabilities {
    /// How many sessions may run at once, as the configuration decides it.
    pub max_streams: u32,
    /// Every device that can render a stream.
    pub devices: Vec<EntertainmentDevice>,
    /// Every entertainment configuration, streaming or not.
    pub configurations: Vec<EntertainmentConfigurationSummary>,
}

/// A device that can render entertainment, with what it offers.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentDevice {
    /// The device's entertainment service.
    pub id: Uuid,
    /// The name the Hue app shows.
    pub name: String,
    /// How many segments the device can address, one for a plain bulb.
    pub segments: u32,
    /// Whether the segments can be laid out by hand.
    pub configurable: bool,
    /// The light service behind it, which is what a zone names.
    pub light: Option<Uuid>,
}

/// An entertainment configuration, as a picker needs it.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentConfigurationSummary {
    pub id: Uuid,
    pub name: String,
    /// `screen`, `monitor`, `music`, `3dspace` or `other`.
    pub configuration_type: String,
    /// `active` while it streams, `inactive` otherwise.
    pub status: String,
    /// The channels it defines, which is how many colours a frame carries.
    pub channels: usize,
    /// The light services in it.
    pub lights: Vec<Uuid>,
}

/// One zone, as the editor reads it.
///
/// The channel list is derived on the server from the members and their
/// positions, so the editor never edits channels: it moves positions and the
/// channels follow.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentZone {
    pub id: Uuid,
    pub name: String,
    /// `screen`, `monitor`, `music`, `3dspace` or `other`.
    pub configuration_type: String,
    /// `active` while it streams, `inactive` otherwise.
    pub status: String,
    /// The app streaming into this zone, while one is.
    pub active_streamer: Option<Uuid>,
    /// One entry per service the zone names, with its positions.
    pub locations: Vec<EntertainmentZoneLocation>,
    /// One entry per channel, in channel order.
    pub channels: Vec<EntertainmentZoneChannel>,
}

/// A service the zone names, and where its segments sit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentZoneLocation {
    /// The entertainment service, which is what a zone names.
    pub service: Uuid,
    /// One position per segment, in the order the device addresses them.
    pub positions: Vec<EntertainmentPosition>,
}

/// One channel: a colour in every frame, and where it belongs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentZoneChannel {
    pub channel_id: u32,
    pub position: EntertainmentPosition,
    pub members: Vec<EntertainmentZoneMember>,
}

/// One segment of one service, as a channel addresses it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentZoneMember {
    /// The entertainment service the segment belongs to.
    pub service: Uuid,
    /// The segment's index on that device.
    pub index: u16,
}

/// A point in the Hue coordinate system.
///
/// `x` runs left to right, `y` bottom to top, `z` front to back, with the
/// origin at the centre of the screen.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentPosition {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// What every stream is doing, as the diagnostics screen reads it.
///
/// The screen asks once a second; nothing here changes per frame.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntertainmentStatus {
    /// How many sessions may run at once.
    pub max_streams: u32,
    /// Every stream that is running, by area.
    pub streams: Vec<StreamStats>,
}
