//! The bridge's own state under `/bifrost/state`.
//!
//! `GET /state/summary` is what the maintenance tab shows: how much the
//! bridge holds, which bridge it is, and which version it runs.
//! `POST /state/reset` empties it, and needs the bridge id typed as the
//! confirmation. Neither touches the configuration.
//!
//! Setup mode has no state and does not mount these routes at all.

use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hyper::StatusCode;

use bifrost_api::admin::{StateCounts, StateResetRequest, StateSummary};
use hue::api::RType;

use crate::routes::bifrost::{BifrostApiResult, refuse};
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;

const YAML_CONTENT_TYPE: &str = "application/yaml";

/// The summary of `state`: counts, and which bridge this is.
async fn summary(state: &AppState) -> BifrostApiResult<StateSummary> {
    let config = state.config();
    let version = state
        .updater()
        .lock()
        .await
        .get()
        .await
        .get_software_version();
    let res = state.res.lock().await;

    let count = |kind: RType| res.get_resources_by_type(kind).len();

    Ok(StateSummary {
        bridge_id: hue::bridge_id(config.bridge.mac),
        mac: config.bridge.mac.to_string(),
        name: config.bridge.name.clone(),
        version,
        counts: StateCounts {
            lights: count(RType::Light),
            grouped_lights: count(RType::GroupedLight),
            rooms: count(RType::Room),
            devices: count(RType::Device),
            scenes: count(RType::Scene),
            entertainment_configurations: count(RType::EntertainmentConfiguration),
        },
    })
}

async fn get_summary(State(state): State<AppState>) -> BifrostApiResult<Json<StateSummary>> {
    Ok(Json(summary(&state).await?))
}

/// The state as a YAML document, for the backup the reset screen offers.
///
/// Nothing downloads this on its own: the user asks for it, then resets.
async fn get_export(State(state): State<AppState>) -> BifrostApiResult<Response> {
    let yaml = state.res.lock().await.serialize()?;
    Ok(([(CONTENT_TYPE, YAML_CONTENT_TYPE)], yaml).into_response())
}

/// Empty the state, after the user typed the bridge id.
///
/// Scenes, room names and everything the z2m backends imported are gone
/// afterwards; the z2m backends re-import their devices from scratch.
async fn post_reset(
    State(state): State<AppState>,
    Json(req): Json<StateResetRequest>,
) -> BifrostApiResult<Response> {
    let config = state.config();
    let bridge_id = hue::bridge_id(config.bridge.mac);

    if req.confirm != bridge_id {
        return Ok(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("Type the bridge id ({bridge_id}) to reset the state"),
        ));
    }

    {
        let mut res = state.res.lock().await;
        res.reset(&bridge_id)?;
    }

    log::warn!("The bridge state was reset through the admin API");

    // the backends hold the resources they imported; a restart re-imports
    let mut mgr = state.manager();
    for name in config.z2m.servers.keys() {
        if mgr.resolve(format!("z2m@{name}")).await.is_ok() {
            mgr.restart(format!("z2m@{name}")).await?;
        }
    }

    Ok(Json(summary(&state).await?).into_response())
}

/// The state routes, mounted by the `/bifrost` nest and therefore behind
/// the same guard as every other admin route.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/summary", get(get_summary))
        .route("/export", get(get_export))
        .route("/reset", post(post_reset))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use hyper::StatusCode;
    use serde_json::{Value, json};
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use hue::api::{Room, RoomArchetype, RoomMetadata};

    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::state::router;
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    /// The bridge id for `00:11:22:33:44:55`.
    const BRIDGE_ID: &str = "001122fffe334455";

    async fn fixture() -> (TempDir, AppState, Router) {
        let dir = TempDir::new();
        let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();
        let router = router().with_state(state.clone());
        (dir, state, router)
    }

    async fn call(
        router: &Router,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String) {
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body)
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    /// Add one room to the state, so a reset has something to remove.
    async fn add_room(state: &AppState) {
        let mut res = state.res.lock().await;
        let link = hue::api::RType::Room.deterministic("office");
        res.add(
            &link,
            hue::api::Resource::Room(Room {
                children: std::collections::BTreeSet::new(),
                metadata: RoomMetadata {
                    name: "Office".into(),
                    archetype: RoomArchetype::Office,
                },
                services: std::collections::BTreeSet::new(),
            }),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn summary_reports_the_bridge_and_its_counts() {
        let (_dir, state, router) = fixture().await;
        add_room(&state).await;

        let (status, body) = call(&router, Method::GET, "/summary", None).await;

        assert_eq!(status, StatusCode::OK);
        let summary: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(summary["bridge_id"], BRIDGE_ID);
        assert_eq!(summary["mac"], "00:11:22:33:44:55");
        assert_eq!(summary["name"], "Bifrost");
        assert_ne!(summary["version"].as_str().unwrap(), "");
        assert_eq!(summary["counts"]["rooms"], 1);
        assert_eq!(summary["counts"]["lights"], 0);
        // the bridge itself and its bridge home
        assert_eq!(summary["counts"]["devices"], 2);
    }

    #[tokio::test]
    async fn reset_needs_the_bridge_id_typed() {
        let (_dir, state, router) = fixture().await;
        add_room(&state).await;

        let (status, body) = call(
            &router,
            Method::POST,
            "/reset",
            Some(json!({"confirm": "yes"})),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body.contains(BRIDGE_ID), "{body}");
        assert_eq!(
            state
                .res
                .lock()
                .await
                .get_resources_by_type(hue::api::RType::Room)
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn reset_empties_the_state_and_keeps_the_bridge() {
        let (_dir, state, router) = fixture().await;
        add_room(&state).await;

        let (status, body) = call(
            &router,
            Method::POST,
            "/reset",
            Some(json!({"confirm": BRIDGE_ID})),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let summary: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(summary["counts"]["rooms"], 0);
        assert_eq!(summary["counts"]["devices"], 2);
        assert_eq!(summary["bridge_id"], BRIDGE_ID);
    }

    #[tokio::test]
    async fn export_hands_out_the_state_document() {
        let (_dir, state, router) = fixture().await;
        add_room(&state).await;

        let (status, body) = call(&router, Method::GET, "/export", None).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Office"), "{body}");
    }
}
