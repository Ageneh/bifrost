pub mod backend;
pub mod config;
pub mod service;
pub mod websocket;

#[cfg(test)]
mod tests;

use std::error::Error;

use axum::Router;
use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use hyper::StatusCode;
use serde::Serialize;
use serde_json::json;

use bifrost_api::admin::ErrorResponse;

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

/// Stopgap until authentication lands: refuses every request under
/// `/bifrost`. The next commit replaces it with the session check.
pub async fn require_admin(_req: Request, _next: Next) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "The Bifrost admin API requires authentication, which is not available yet"
                .to_string(),
            errors: vec![],
        }),
    )
        .into_response()
}

async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "No such endpoint".to_string(),
            errors: vec![],
        }),
    )
        .into_response()
}

/// Every route under `/bifrost`, all behind [`require_admin`].
///
/// The guard is a `layer` over the whole nest, fallback included, so a route
/// added here can never be mounted outside it.
pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/service", service::router())
        .nest("/backend", backend::router())
        .nest("/config", config::router())
        .route("/ws", any(websocket))
        .fallback(not_found)
        .layer(middleware::from_fn(require_admin))
}
