//! Entertainment under `/bifrost/entertainment`.
//!
//! What this bridge can stream on, and what is streaming. Nothing here starts
//! a stream: `EntertainmentStart` is issued by the DTLS handler the moment a
//! sender's first frame names an area, so a Start button could only announce
//! frames no client will ever send. The UI says the TV or the app starts it.

use axum::Router;
use axum::extract::State;
use axum::routing::get;

use bifrost_api::entertainment::{
    EntertainmentCapabilities, EntertainmentConfigurationSummary, EntertainmentDevice,
};

use crate::routes::bifrost::BifrostApiResult;
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;
use hue::api::{Device, Entertainment, EntertainmentConfiguration, RType};

/// Every device that can render entertainment and every configuration there
/// is, with the limit the configuration sets.
async fn capabilities(
    State(state): State<AppState>,
) -> BifrostApiResult<Json<EntertainmentCapabilities>> {
    let (devices, configurations) = {
        let res = state.res.lock().await;

        let mut devices = vec![];

        for id in res.get_resource_ids_by_type(RType::Entertainment) {
            let Ok(ent) = res.get_id::<Entertainment>(id) else {
                continue;
            };

            // the bridge's own proxy carries the stream limit and renders
            // nothing; it is not a device a zone can be built from
            if ent.renderer_reference.is_none() && ent.max_streams.is_some() {
                continue;
            }

            let name = res
                .get::<Device>(&ent.owner)
                .map(|dev| dev.metadata.name.clone())
                .unwrap_or_default();

            let (segments, configurable) = ent.segments.as_ref().map_or((1, false), |segments| {
                (segments.max_segments, segments.configurable)
            });

            devices.push(EntertainmentDevice {
                id,
                name,
                segments,
                configurable,
                light: ent.renderer_reference.map(|link| link.rid),
            });
        }

        devices.sort_by(|a, b| a.name.cmp(&b.name));

        let mut configurations = vec![];

        for id in res.get_resource_ids_by_type(RType::EntertainmentConfiguration) {
            let Ok(ec) = res.get_id::<EntertainmentConfiguration>(id) else {
                continue;
            };

            configurations.push(EntertainmentConfigurationSummary {
                id,
                name: ec.metadata.name.clone(),
                configuration_type: serde_json::to_value(&ec.configuration_type)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default(),
                status: serde_json::to_value(ec.status)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default(),
                channels: ec.channels.len(),
                lights: ec.light_services.iter().map(|link| link.rid).collect(),
            });
        }

        configurations.sort_by(|a, b| a.name.cmp(&b.name));

        drop(res);

        (devices, configurations)
    };

    let max_streams = state.config().bifrost.entertainment.max_streams;

    Ok(Json(EntertainmentCapabilities {
        max_streams,
        devices,
        configurations,
    }))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/capabilities", get(capabilities))
}
