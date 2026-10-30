pub mod auth;
pub mod certificate;
pub mod config;
pub mod devices;
pub mod identity;
pub mod logs;
pub mod pairing;
pub mod rooms;
pub mod service;
pub mod setup;
pub mod state;
pub mod websocket;
pub mod wizard;
pub mod z2m;

#[cfg(test)]
pub mod tests;

use std::error::Error;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::Method;
use axum::http::header::{HOST, ORIGIN, REFERER, UPGRADE};
use axum::http::request::Parts;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use hyper::StatusCode;
use serde::Serialize;
use serde_json::json;
use url::Url;

use bifrost_api::admin::ErrorResponse;
use bifrost_api::admin::auth::CSRF_HEADER;

use crate::auth::{Auth, secrets_match};
use crate::config::{AppConfig, AuthConfig};
use crate::routes::bifrost::auth::{Client, TrustedIngress};
use crate::routes::bifrost::websocket::websocket;
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;
use crate::server::mdns;

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

/// Requests through Home Assistant Ingress carry this header. It is only
/// believed from a trusted peer address.
const INGRESS_HEADER: &str = "x-ingress-path";

/// Routes that answer without a session: they are how one is obtained.
const PUBLIC_ROUTES: &[(Method, &str)] = &[
    (Method::GET, "/auth/session"),
    (Method::POST, "/auth/login"),
    (Method::POST, "/auth/setup"),
];

#[must_use]
pub fn refuse(status: StatusCode, error: &str) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
            errors: vec![],
        }),
    )
        .into_response()
}

/// Host and optional port of an authority such as `host:80` or `[::1]:80`.
fn split_authority(authority: &str) -> (String, Option<u16>) {
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => (host, port.parse().ok()),
        _ => (authority, None),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (host.to_ascii_lowercase(), port)
}

/// The authority the client addressed: the `Host` header, or the HTTP/2
/// `:authority`.
fn request_authority(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| parts.uri.authority().map(ToString::to_string))
}

/// Host names the admin interface answers to: the bridge address, its mDNS
/// name, and the configured extras. Refusing every other name is what stops
/// DNS rebinding, where an attacker's domain resolves to this bridge.
fn bridge_host_allowed(host: &str, config: &AppConfig) -> bool {
    host == config.bridge.ipaddress.to_string()
        || host == mdns::host_name(config.bridge.mac)
        || config
            .bifrost
            .auth
            .allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
}

/// The origin a browser says the request comes from: `Origin`, or the
/// origin of `Referer` when a browser left `Origin` out.
fn request_origin(parts: &Parts) -> Option<Url> {
    let header = |name| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    header(ORIGIN)
        .or_else(|| header(REFERER))
        .and_then(|origin| Url::parse(origin).ok())
}

/// A mutation, or a websocket upgrade, must come from a page served by this
/// bridge: same host and same port.
fn origin_allowed(parts: &Parts, host_allowed: &dyn Fn(&str) -> bool) -> bool {
    let (Some(origin), Some(authority)) = (request_origin(parts), request_authority(parts)) else {
        return false;
    };
    let Some(origin_host) = origin.host_str() else {
        return false;
    };
    let origin_host = origin_host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let (host, port) = split_authority(&authority);

    // a browser leaves the default port out of both; an origin that names a
    // port the host does not is another service on this host, e.g. Home
    // Assistant on :8123
    let same_port = port.map_or_else(
        || origin.port().is_none(),
        |port| origin.port_or_known_default() == Some(port),
    );

    matches!(origin.scheme(), "http" | "https")
        && origin_host == host
        && host_allowed(&host)
        && same_port
}

fn is_websocket_upgrade(parts: &Parts) -> bool {
    parts
        .headers
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

/// The access decision for every route under `/bifrost`, in running and in
/// setup mode.
///
/// In order: an Ingress claim is refused unless it comes from a trusted
/// peer, which then skips the login; the `Host` must be one of this
/// bridge's names; a session is required except on [`PUBLIC_ROUTES`]; and
/// every mutation and websocket upgrade needs an allowed `Origin`, a
/// mutation with a session also the session's `X-Bifrost-Csrf`.
///
/// Returns the refusal, or `None` to let the request through.
pub fn check_access(
    parts: &mut Parts,
    sessions: &Auth,
    auth: &AuthConfig,
    host_allowed: &dyn Fn(&str) -> bool,
) -> Option<Response> {
    let Some(client) = Client::from_parts(parts) else {
        return Some(refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "The client address is unknown",
        ));
    };

    // decided by the connecting address only: every header is client-set
    let trusted_peer = auth.trust_ingress
        && auth
            .trusted_cidrs
            .iter()
            .any(|cidr| cidr.contains(client.ip));
    let ingress = parts.headers.contains_key(INGRESS_HEADER);
    if ingress && !trusted_peer {
        log::warn!(
            "Refused an Ingress request from untrusted peer {}",
            client.ip
        );
        return Some(refuse(
            StatusCode::FORBIDDEN,
            "Ingress requests are only accepted from the Home Assistant Supervisor",
        ));
    }

    if ingress {
        parts.extensions.insert(TrustedIngress);
        return None;
    }

    let host = request_authority(parts).map(|authority| split_authority(&authority).0);
    if !host.is_some_and(|host| host_allowed(&host)) {
        return Some(refuse(
            StatusCode::FORBIDDEN,
            "Unknown host name; add it to bifrost.auth.allowed_hosts to use it",
        ));
    }

    let mutation = !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS);
    let needs_origin = mutation || is_websocket_upgrade(parts);
    let public = PUBLIC_ROUTES
        .iter()
        .any(|(method, path)| parts.method == method && parts.uri.path() == *path);

    if public {
        if needs_origin && !origin_allowed(parts, host_allowed) {
            return Some(refuse(
                StatusCode::FORBIDDEN,
                "Cross-origin request refused",
            ));
        }
        return None;
    }

    let Some(csrf) = client.session(sessions, auth) else {
        return Some(refuse(
            StatusCode::UNAUTHORIZED,
            "Log in to use the admin API",
        ));
    };

    if needs_origin && !origin_allowed(parts, host_allowed) {
        return Some(refuse(
            StatusCode::FORBIDDEN,
            "Cross-origin request refused",
        ));
    }

    if mutation {
        let presented = parts
            .headers
            .get(CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !secrets_match(presented, &csrf) {
            return Some(refuse(
                StatusCode::FORBIDDEN,
                "Missing or wrong X-Bifrost-Csrf header",
            ));
        }
    }

    None
}

/// The access guard over every route under `/bifrost`; see
/// [`check_access`].
///
/// The request passes through untouched, so the websocket upgrade still
/// works behind it.
pub async fn require_admin(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let config = state.config();
    let host_allowed = |host: &str| bridge_host_allowed(host, &config);

    match check_access(
        &mut parts,
        &state.auth(),
        &config.bifrost.auth,
        &host_allowed,
    ) {
        None => next.run(Request::from_parts(parts, body)).await,
        Some(refusal) => refusal,
    }
}

pub async fn not_found() -> Response {
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
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/service", service::router())
        .nest("/bridge", pairing::router())
        .nest("/bridge/certificate", certificate::router())
        .nest("/bridge/identity", identity::router())
        .nest("/devices", devices::router())
        .nest("/z2m", z2m::router())
        .nest("/config", config::router())
        .nest("/logs", logs::router())
        .nest("/rooms", rooms::router())
        .nest("/setup", wizard::router())
        .nest("/state", state::router())
        .route("/ws", any(websocket))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(state, require_admin))
}
