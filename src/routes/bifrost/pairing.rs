//! The pairing screen's backend under `/bifrost/bridge`.
//!
//! Bifrost has no physical link button, so the screen opens a window in its
//! place: `POST /link-button` opens one, `GET /link-button` reports what is
//! left of it, and `GET /pairing` returns the mode and the attempts so far.
//!
//! The window is a convenience, not access control. A client that pairs
//! while it is open receives the same fixed username and DTLS key as every
//! other client, so the history is a log, not a list that can be revoked.

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use hyper::StatusCode;

use bifrost_api::admin::{LinkButtonPress, LinkButtonStatus, PairingHistory};

use crate::routes::bifrost::{BifrostApiResult, refuse};
use crate::routes::extractor::Json;
use crate::server::appstate::{AppState, DEFAULT_WINDOW_SECONDS, MAX_WINDOW_SECONDS};

/// The window as it stands right now.
async fn get_link_button(State(state): State<AppState>) -> Json<LinkButtonStatus> {
    Json(state.link_button().status())
}

/// Open a window, replacing one that is already open.
///
/// A duration above [`MAX_WINDOW_SECONDS`] is refused rather than clamped:
/// a screen that asks for ten minutes is a bug, and silently giving it two
/// would hide that.
async fn post_link_button(
    State(state): State<AppState>,
    bytes: Bytes,
) -> BifrostApiResult<Response> {
    // an empty body means "the default window", which is what the button on
    // the screen sends when the user does not pick a duration
    let req: LinkButtonPress = if bytes.is_empty() {
        LinkButtonPress { seconds: None }
    } else {
        serde_json::from_slice(&bytes)?
    };
    let seconds = req.seconds.unwrap_or(DEFAULT_WINDOW_SECONDS);

    if seconds == 0 || seconds > MAX_WINDOW_SECONDS {
        return Ok(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("Open the pairing window for 1 to {MAX_WINDOW_SECONDS} seconds"),
        ));
    }

    let status = state.link_button().press(seconds);
    log::info!("Pairing window opened for {seconds} s");

    Ok(Json(status).into_response())
}

/// The mode and the pairing attempts the bridge remembers.
async fn get_pairing(State(state): State<AppState>) -> Json<PairingHistory> {
    Json(PairingHistory {
        mode: state.pairing_mode(),
        records: state.link_button().history(),
    })
}

/// The pairing routes, mounted by the `/bifrost` nest and therefore behind
/// the same guard as every other admin route.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/link-button", get(get_link_button).post(post_link_button))
        .route("/pairing", get(get_pairing))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use hyper::StatusCode;
    use serde_json::Value;
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::pairing::router;
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, ManualClock, TempDir};

    async fn fixture() -> (TempDir, Arc<ManualClock>, AppState, Router) {
        let dir = TempDir::new();
        let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let clock = ManualClock::new();
        let state = AppState::from_store(Arc::new(store), svm)
            .await
            .unwrap()
            .with_link_button_clock(clock.clone());
        let router = router().with_state(state.clone());
        (dir, clock, state, router)
    }

    async fn call(
        router: &Router,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
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
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn a_window_counts_down_and_closes_on_its_own() {
        let (_dir, clock, _state, router) = fixture().await;

        let (status, body) = call(
            &router,
            Method::POST,
            "/link-button",
            Some(serde_json::json!({"seconds": 5})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["pressed"], true);
        assert_eq!(body["remaining_seconds"], 5);

        clock.advance(chrono::TimeDelta::seconds(3));
        let (_, body) = call(&router, Method::GET, "/link-button", None).await;
        assert_eq!(body["pressed"], true);
        assert_eq!(body["remaining_seconds"], 2);

        clock.advance(chrono::TimeDelta::seconds(3));
        let (_, body) = call(&router, Method::GET, "/link-button", None).await;
        assert_eq!(body["pressed"], false);
        assert_eq!(body["remaining_seconds"], 0);
    }

    #[tokio::test]
    async fn the_default_window_is_thirty_seconds() {
        let (_dir, _clock, _state, router) = fixture().await;

        let (_, body) = call(&router, Method::POST, "/link-button", None).await;

        assert_eq!(body["remaining_seconds"], 30);
    }

    #[tokio::test]
    async fn a_window_above_the_maximum_is_refused() {
        let (_dir, _clock, _state, router) = fixture().await;

        let (status, body) = call(
            &router,
            Method::POST,
            "/link-button",
            Some(serde_json::json!({"seconds": 121})),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].as_str().unwrap().contains("120"), "{body}");
        let (_, status) = call(&router, Method::GET, "/link-button", None).await;
        assert_eq!(status["pressed"], false);
    }

    #[tokio::test]
    async fn history_reports_the_mode_and_the_attempts() {
        let (_dir, _clock, state, router) = fixture().await;
        state.link_button().record("Hue Sync Box", false);

        let (_, body) = call(&router, Method::GET, "/pairing", None).await;

        assert_eq!(body["mode"], "compat");
        assert_eq!(body["records"][0]["devicetype"], "Hue Sync Box");
        assert_eq!(body["records"][0]["accepted"], false);
    }
}
