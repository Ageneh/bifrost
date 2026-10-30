//! What devices just reported, as the console's Events strip shows it.
//!
//! A reading also lands in the inventory, which is the current state; this
//! channel is the *change*, and a screen that shows "21.4 °C at 14:02" needs
//! the second one. The two are published from the same message so they can
//! never disagree, and neither is a second path to the Hue API: the event
//! diff there stays automatic.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::broadcast;

use bifrost_api::websocket::DeviceEvent;

/// How many events a slow subscriber may fall behind before it is told it
/// missed some. A screen that lags redraws from the inventory.
const EVENT_BUFFER: usize = 256;

/// The fan-out every device report is published to.
#[derive(Debug)]
pub struct DeviceEvents {
    events: broadcast::Sender<DeviceEvent>,
}

impl DeviceEvents {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Arc::new(Self { events })
    }

    /// Follow every report.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<DeviceEvent> {
        self.events.subscribe()
    }

    /// Publish one report.
    pub fn publish(&self, ieee: &str, kind: &str, value: Value, at: DateTime<Utc>) {
        // a send error only means nobody is listening
        let _ = self.events.send(DeviceEvent {
            ieee: ieee.to_string(),
            kind: kind.to_string(),
            value,
            at,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_reaches_every_subscriber_with_its_fields() {
        let events = DeviceEvents::new();
        let mut first = events.subscribe();
        let mut second = events.subscribe();
        let at = Utc::now();

        events.publish(
            "0x00158d0000000020",
            "occupancy",
            serde_json::json!(true),
            at,
        );

        for receiver in [&mut first, &mut second] {
            let event = receiver.try_recv().unwrap();
            assert_eq!(event.ieee, "0x00158d0000000020");
            assert_eq!(event.kind, "occupancy");
            assert_eq!(event.value, serde_json::json!(true));
            assert_eq!(event.at, at);
        }
    }

    #[test]
    fn publishing_with_no_subscriber_is_not_an_error() {
        DeviceEvents::new().publish("0x01", "action", serde_json::json!("single"), Utc::now());
    }
}
