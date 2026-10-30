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
