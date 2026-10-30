//! Rooms under `/bifrost/rooms`.
//!
//! A room is the z2m group the backend imported, so its membership belongs
//! to the mesh. The screen drafts a new membership into the configuration
//! (`rooms.<topic>.devices`) and apply sends the difference; nothing here
//! writes to the mesh. Scenes are listed and recalled only: authoring stays
//! in the Hue app.

use axum::Router;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde_json::Value;

use bifrost_api::admin::{RoomDevice, RoomEntry, RoomList, RoomScene, SceneRecallRequest};
use bifrost_api::backend::BackendRequest;
use hue::api::{RType, Room, Scene, SceneActive, SceneRecall, SceneStatusEnum, SceneUpdate};

use crate::routes::bifrost::{BifrostApiResult, not_found, refuse};
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;

/// Every room, with the devices in it and the scenes it holds.
async fn list(State(state): State<AppState>) -> BifrostApiResult<Json<RoomList>> {
    let inventory = state.inventory().snapshot();
    let res = state.res.lock().await;

    let mut rooms = vec![];

    for rid in res.get_resource_ids_by_type(RType::Room) {
        let Ok(room) = res.get_id::<Room>(rid) else {
            continue;
        };

        // the backend records the group topic beside the room
        let link = RType::Room.link_to(rid);
        let topic = res
            .aux_get(&link)
            .ok()
            .and_then(|aux| aux.topic.clone())
            .unwrap_or_else(|| room.metadata.name.clone());

        let icon = serde_json::to_value(room.metadata.archetype)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default();

        let devices = room
            .children
            .iter()
            .filter_map(|link| {
                let device = inventory.devices.iter().find(|device| {
                    device
                        .resources
                        .iter()
                        .any(|r| r.rid == link.rid.to_string())
                })?;
                Some(RoomDevice {
                    ieee: device.ieee.clone(),
                    name: device.friendly_name.clone(),
                    supported: device.supported,
                })
            })
            .collect();

        let scenes = res
            .get_scenes_for_room(&rid)
            .into_iter()
            .filter_map(|scene_rid| {
                let scene = res.get_id::<Scene>(scene_rid).ok()?;
                Some(RoomScene {
                    id: scene_rid.to_string(),
                    name: scene.metadata.name.clone(),
                    active: !matches!(
                        scene.status.as_ref().map(|status| status.active),
                        None | Some(SceneActive::Inactive)
                    ),
                })
            })
            .collect();

        rooms.push(RoomEntry {
            id: rid.to_string(),
            topic,
            name: room.metadata.name.clone(),
            icon,
            devices,
            scenes,
        });
    }

    rooms.sort_by(|a, b| a.name.cmp(&b.name));
    drop(res);

    Ok(Json(RoomList { rooms }))
}

/// Recall one of a room's scenes, the same request a v2 scene `PUT` sends.
async fn recall(
    State(state): State<AppState>,
    Path((room, scene)): Path<(String, String)>,
    Json(req): Json<SceneRecallRequest>,
) -> BifrostApiResult<axum::response::Response> {
    let Ok(rid) = room.parse::<uuid::Uuid>() else {
        return Ok(not_found().await);
    };
    let Ok(sid) = scene.parse::<uuid::Uuid>() else {
        return Ok(not_found().await);
    };

    let link = RType::Scene.link_to(sid);
    let res = state.res.lock().await;

    let Ok(found) = res.get::<Scene>(&link) else {
        return Ok(not_found().await);
    };
    if found.group.rid != rid {
        return Ok(refuse(
            hyper::StatusCode::BAD_REQUEST,
            "That scene belongs to another room",
        ));
    }

    let update = SceneUpdate {
        recall: Some(SceneRecall {
            action: Some(SceneStatusEnum::Active),
            duration: req.duration,
            dimming: None,
        }),
        ..SceneUpdate::default()
    };

    res.backend_request(BackendRequest::SceneUpdate(link, update))?;
    drop(res);

    Ok(Json(Value::Null).into_response())
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/{room}/scenes/{scene}/recall", post(recall))
}
