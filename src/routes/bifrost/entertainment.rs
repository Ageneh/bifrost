//! Entertainment under `/bifrost/entertainment`.
//!
//! What this bridge can stream on, and what is streaming. Nothing here starts
//! a stream: `EntertainmentStart` is issued by the DTLS handler the moment a
//! sender's first frame names an area, so a Start button could only announce
//! frames no client will ever send. The UI says the TV or the app starts it.

use axum::Router;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::Value;
use uuid::Uuid;

use bifrost_api::backend::BackendRequest;
use bifrost_api::entertainment::{
    EntertainmentCapabilities, EntertainmentConfigurationSummary, EntertainmentDevice,
    EntertainmentPosition, EntertainmentStatus, EntertainmentZone, EntertainmentZoneChannel,
    EntertainmentZoneLocation, EntertainmentZoneMember,
};

use crate::routes::bifrost::{BifrostApiResult, not_found};
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

/// What every stream is doing.
///
/// The screen asks once a second. The rates in the answer are measured over
/// that second, and nothing is computed per frame.
async fn status(State(state): State<AppState>) -> BifrostApiResult<Json<EntertainmentStatus>> {
    Ok(Json(status_of(&state)))
}

/// The status, as both this route and the admin socket report it.
#[must_use]
pub fn status_of(state: &AppState) -> EntertainmentStatus {
    EntertainmentStatus {
        max_streams: state.config().bifrost.entertainment.max_streams,
        streams: state.streams().snapshot(),
    }
}

/// End the stream on one area.
///
/// The session ends on its next frame and the backend stops that area's
/// lights and status. The stop is sent here as well, so the lights answer
/// the request at once rather than up to a frame later.
async fn stop(State(state): State<AppState>, Path(id): Path<String>) -> BifrostApiResult<Response> {
    let Ok(area) = id.parse::<Uuid>() else {
        return Ok(not_found().await);
    };

    if !state.streams().request_stop(&area) {
        return Ok(not_found().await);
    }

    state
        .res
        .lock()
        .await
        .backend_request(BackendRequest::EntertainmentStop(area))?;

    Ok(Json(Value::Null).into_response())
}

/// One zone, with its positions and its derived channels.
///
/// The editor reads this rather than the CLIP resource: the same numbers, in
/// the shape the screen needs, behind the admin session like every other
/// read here.
async fn configuration(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> BifrostApiResult<Response> {
    let Ok(zone) = id.parse::<Uuid>() else {
        return Ok(not_found().await);
    };

    let res = state.res.lock().await;
    let Ok(ec) = res.get_id::<EntertainmentConfiguration>(zone) else {
        return Ok(not_found().await);
    };

    let locations = ec
        .locations
        .service_locations
        .iter()
        .map(|location| EntertainmentZoneLocation {
            service: location.service.rid,
            positions: location.positions.iter().map(position_of).collect(),
        })
        .collect();

    let channels = ec
        .channels
        .iter()
        .map(|channel| EntertainmentZoneChannel {
            channel_id: channel.channel_id,
            position: position_of(&channel.position),
            members: channel
                .members
                .iter()
                .map(|member| EntertainmentZoneMember {
                    service: member.service.rid,
                    index: member.index,
                })
                .collect(),
        })
        .collect();

    Ok(Json(EntertainmentZone {
        id: zone,
        name: ec.metadata.name.clone(),
        configuration_type: serde_json::to_value(&ec.configuration_type)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default(),
        status: serde_json::to_value(&ec.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default(),
        active_streamer: ec.active_streamer.map(|link| link.rid),
        locations,
        channels,
    })
    .into_response())
}

fn position_of(position: &hue::api::Position) -> EntertainmentPosition {
    EntertainmentPosition {
        x: position.x,
        y: position.y,
        z: position.z,
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/capabilities", get(capabilities))
        .route("/status", get(status))
        .route("/configurations/{id}", get(configuration))
        .route("/{id}/stop", post(stop))
}
