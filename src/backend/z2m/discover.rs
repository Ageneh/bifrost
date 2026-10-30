//! One look at a z2m server, for the first-run wizard.
//!
//! Before the bridge exists there is no backend to ask, so the wizard dials
//! the server itself, reads the devices it announces, and hangs up. Nothing
//! is imported and nothing is written: this is a question, not a
//! configuration.

use std::time::Duration;

use bifrost_api::admin::{DiscoveredDevice, DiscoveredRoom};
use native_tls::TlsConnector;
use tokio_tungstenite::Connector;
use tokio_tungstenite::connect_async_tls_with_config;
use z2m::api::RawMessage;

use crate::config::Z2mServer;
use crate::error::{ApiError, ApiResult};

/// How long the server has to answer with its device list.
const PATIENCE: Duration = Duration::from_secs(10);

/// The devices `server` announces, as Bifrost would import them.
///
/// # Errors
///
/// Fails when the server cannot be reached, or never sends `bridge/devices`.
// the house error type is what the rest of the backend returns, so it is
// propagated rather than flattened here
#[allow(clippy::result_large_err)]
pub async fn devices(
    server: &Z2mServer,
) -> ApiResult<(Vec<DiscoveredDevice>, Vec<DiscoveredRoom>)> {
    let url = server.get_url();

    // the same certificate rule the backend applies: a server the user
    // marked as self-signed is accepted here too, and nowhere else
    let connector = if server.disable_tls_verify.unwrap_or_default() {
        Some(Connector::NativeTls(
            TlsConnector::builder()
                .danger_accept_invalid_certs(true)
                .build()
                .map_err(|err| ApiError::Other(err.to_string()))?,
        ))
    } else {
        None
    };

    let (mut socket, _) = connect_async_tls_with_config(url.as_str(), None, false, connector)
        .await
        .map_err(|err| ApiError::Other(format!("cannot reach {url}: {err}")))?;

    tokio::time::timeout(PATIENCE, async {
        use futures::StreamExt;

        let mut devices = None;
        let mut rooms = None;

        while let Some(message) = socket.next().await {
            let Ok(message) = message else {
                return Err(ApiError::Other("the server hung up".to_string()));
            };
            let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
                continue;
            };
            let Ok(raw) = serde_json::from_str::<RawMessage>(&text) else {
                continue;
            };

            match raw.topic.as_str() {
                "bridge/devices" => devices = Some(read_devices(&raw.payload)),
                "bridge/groups" => rooms = Some(read_rooms(&raw.payload)),
                _ => continue,
            }

            // a server announces both; waiting for the second keeps the
            // wizard from showing a roomless first run
            if devices.is_some() && rooms.is_some() {
                break;
            }
        }

        let _ = socket.close(None).await;

        devices.map_or_else(
            || {
                Err(ApiError::Other(
                    "the server sent no device list".to_string(),
                ))
            },
            |devices| Ok((devices, rooms.unwrap_or_default())),
        )
    })
    .await
    .map_err(|_| ApiError::Other(format!("no answer from {url} within {PATIENCE:?}")))?
}

/// The rooms in a `bridge/groups` payload.
fn read_rooms(payload: &serde_json::Value) -> Vec<DiscoveredRoom> {
    let Some(groups) = payload.as_array() else {
        return vec![];
    };

    groups
        .iter()
        .filter_map(|group| {
            let topic = group.get("friendly_name")?.as_str()?.to_string();
            let members = group
                .get("members")
                .and_then(serde_json::Value::as_array)
                .map(|members| {
                    members
                        .iter()
                        .filter_map(|member| {
                            member
                                .get("ieee_address")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default();

            Some(DiscoveredRoom { topic, members })
        })
        .collect()
}

/// The devices in a `bridge/devices` payload.
fn read_devices(payload: &serde_json::Value) -> Vec<DiscoveredDevice> {
    let Some(devices) = payload.as_array() else {
        return vec![];
    };

    devices
        .iter()
        .filter_map(|device| {
            let ieee = device.get("ieee_address")?.as_str()?;
            let name = device.get("friendly_name")?.as_str()?;

            // a coordinator is not a light, and the wizard has nothing to
            // offer for it
            if device.get("type").and_then(serde_json::Value::as_str) == Some("Coordinator") {
                return None;
            }

            let definition = device.get("definition");
            let model = definition
                .and_then(|def| def.get("model"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let vendor = definition
                .and_then(|def| def.get("vendor"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);

            // the same rule the import uses: the class the inventory
            // derives decides whether the Hue API can serve the device, so
            // a sensor the wizard offers is a sensor it can import
            // parsed from text, not a `Value`: the IEEE address deserializer
            // borrows the string it reads
            let supported = serde_json::from_str::<z2m::api::Device>(&device.to_string())
                .is_ok_and(|device| crate::inventory::classify(&device).1);

            Some(DiscoveredDevice {
                ieee: ieee.to_string(),
                friendly_name: name.to_string(),
                model,
                vendor,
                supported,
            })
        })
        .collect()
}
