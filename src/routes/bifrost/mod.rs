pub mod backend;
pub mod config;
pub mod service;
pub mod websocket;

use std::error::Error;

use axum::Router;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use hyper::StatusCode;
use serde::Serialize;
use serde_json::json;

use crate::routes::bifrost::websocket::websocket;
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;

#[derive(Debug, Serialize)]
/// Simple bifrost api error wrapper.
///
/// Bifrost API results need to implement [`IntoResponse`], but since
/// [`BifrostError`] comes from [`bifrost_api`], we can't implement
/// [`IntoResponse`] for it, without gaining a dependency on [`axum`] for that
/// crate. So for now, we use this thin wrapper for an [`IntoResponse`] impl.
struct BifrostApiError(String);

type BifrostApiResult<T> = Result<T, BifrostApiError>;

impl<E: Error> From<E> for BifrostApiError {
    fn from(value: E) -> Self {
        Self(value.to_string())
    }
}

impl IntoResponse for BifrostApiError {
    fn into_response(self) -> Response {
        log::error!("Request failed: {}", self.0);

        let res = json!({"error": self.0});

        (StatusCode::INTERNAL_SERVER_ERROR, Json(res)).into_response()
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/service", service::router())
        .nest("/backend", backend::router())
        .nest("/config", config::router())
        .route("/ws", any(websocket))
}
