use hue::event::EventBlock;
use serde::{Deserialize, Serialize};

use crate::admin::{LinkButtonStatus, LogRecord};
use crate::backend::BackendRequest;
use crate::config::AppConfig;
use crate::inventory::InventoryUpdate;
use crate::service::Service;

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
}
