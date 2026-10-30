use chrono::{DateTime, Utc};
use hue::event::EventBlock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::admin::{LinkButtonStatus, LogRecord};
use crate::backend::BackendRequest;
use crate::config::AppConfig;
use crate::entertainment::EntertainmentStatus;
use crate::inventory::InventoryUpdate;
use crate::service::Service;

/// One thing a device just reported.
///
/// The console's Events strip reads this instead of the Hue resource model:
/// a sensor's reading and a remote's button press are the two things a
/// person watches for, and neither needs the 215 serde-derived Hue types in
/// the UI's type graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct DeviceEvent {
    /// The IEEE address of the device that reported.
    pub ieee: String,
    /// The property: `occupancy`, `temperature`, `illuminance`, `contact`
    /// or `action`.
    pub kind: String,
    /// The value, in the semantics its class reads.
    pub value: Value,
    /// When the device reported it.
    pub at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Update {
    AppConfig(AppConfig),
    HueEvent(EventBlock),
    BackendRequest(BackendRequest),
    ServiceUpdate(Service),
    /// A new line in the bridge's log buffer.
    Log(LogRecord),
    /// The device inventory: a whole snapshot when a socket opens, then one
    /// change at a time.
    Inventory(InventoryUpdate),
    /// The pairing window opened, was consumed, or is a snapshot.
    LinkButton(LinkButtonStatus),
    /// A device reported something: a sensor reading or a button press.
    DeviceEvent(DeviceEvent),
    /// A stream started, ended or failed. Sent when the set changes, never
    /// per frame: the rates come from polling the status endpoint.
    EntertainmentStatus(EntertainmentStatus),
}
