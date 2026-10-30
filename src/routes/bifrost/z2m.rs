//! z2m server administration under `/bifrost/z2m`.
//!
//! The list reads the configuration the screens edit (the draft when there is
//! one) and the state of each server's backend. Every write goes through the
//! configuration draft, exactly like a hand-written patch: the backend is
//! `z2m@<name>`, so a server is added, changed or removed by applying the
//! draft, never by registering a service here.

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use hyper::StatusCode;
use serde_json::Value;
use svc::traits::ServiceState;

use bifrost_api::admin::{
    DraftResponse, RevisionRequest, Z2mServerEntry, Z2mServerList, Z2mServerWrite,
};

use crate::config::redact::{redact, redacted};
use crate::config::validate::ValidationError;
use crate::routes::bifrost::config::{ConfigApiError, ConfigApiResult, patch_draft};
use crate::routes::bifrost::{BifrostApiResult, refuse};
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;
use crate::service_registry::Z2M_TEMPLATE;

/// The configuration the screens edit: the draft, or the active one.
fn edited_config(state: &AppState) -> crate::config::AppConfig {
    let store = state.store();
    store
        .draft()
        .map_or_else(|| store.active(), |draft| draft.config)
}

/// State of every running backend, by server name.
async fn backend_states(
    state: &AppState,
) -> Result<HashMap<String, ServiceState>, svc::error::SvcError> {
    let mut mgr = state.manager();
    let mut states = HashMap::new();

    for (id, name) in mgr.list().await? {
        if name.name() != Z2M_TEMPLATE {
            continue;
        }
        let Some(instance) = name.instance() else {
            continue;
        };
        // a backend that was just unregistered can still be listed
        if let Ok(service_state) = mgr.status(id).await {
            states.insert(instance.to_string(), service_state);
        }
    }

    Ok(states)
}

async fn list_servers(State(state): State<AppState>) -> BifrostApiResult<Json<Z2mServerList>> {
    let config = edited_config(&state);
    let states = backend_states(&state).await?;

    let mut servers: Vec<Z2mServerEntry> = config
        .z2m
        .servers
        .iter()
        .map(|(name, server)| {
            let mut config = serde_json::to_value(server).unwrap_or(Value::Null);
            redact(&mut config);
            Z2mServerEntry {
                name: name.clone(),
                state: states.get(name).copied(),
                config,
            }
        })
        .collect();
    servers.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(Z2mServerList { servers }))
}

#[allow(clippy::result_large_err)]
async fn get_server(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Z2mServerEntry>, Response> {
    let config = edited_config(&state);
    let Some(server) = config.z2m.servers.get(&name) else {
        return Err(refuse(
            StatusCode::NOT_FOUND,
            &format!("No z2m server is configured as {name:?}"),
        ));
    };

    let states = backend_states(&state)
        .await
        .map_err(|err| refuse(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()))?;

    let state_of = states.get(&name).copied();
    Ok(Json(Z2mServerEntry {
        name,
        state: state_of,
        config: redacted(&serde_json::to_value(server).unwrap_or(Value::Null)),
    }))
}

async fn put_server(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<Z2mServerWrite>,
) -> ConfigApiResult<Json<DraftResponse>> {
    // the backend is the service `z2m@<name>`, so the name has to survive
    // that as one instance name
    if name.is_empty() || name.contains('@') {
        return Err(ConfigApiError::Invalid(vec![ValidationError::error(
            format!("z2m.{name}"),
            "a z2m server name must not be empty or contain '@'",
        )]));
    }

    let patch = serde_json::json!({"z2m": {name: req.server}});
    Ok(Json(patch_draft(&state, req.revision, &patch)?))
}

async fn delete_server(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<RevisionRequest>,
) -> ConfigApiResult<Json<DraftResponse>> {
    // null removes the key, which the apply plan reads as a stopped backend
    let patch = serde_json::json!({"z2m": {name: Value::Null}});
    Ok(Json(patch_draft(&state, req.revision, &patch)?))
}

/// The z2m routes, mounted by the `/bifrost` nest and therefore behind the
/// same guard as every other admin route.
pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_servers)).route(
        "/{name}",
        get(get_server).put(put_server).delete(delete_server),
    )
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

    use crate::config::redact::REDACTED;
    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::z2m::router;
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    const SECRET: &str = "s3cret-token-value";

    struct Fixture {
        _dir: TempDir,
        state: AppState,
        router: Router,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = TempDir::new();
            let yaml = MINIMAL_CONFIG.replace(
                "url: ws://10.0.0.100:8080",
                &format!("url: ws://10.0.0.100:8080/api?token={SECRET}"),
            );
            let store = ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap();
            // the first apply moves the url token into the token field, as
            // every running bridge has done
            store.set_draft(0, store.active()).unwrap();
            store.apply(1).unwrap();
            let (svm, _future) = ServiceManager::spawn();
            let state = AppState::from_store(Arc::new(store), svm).await.unwrap();

            Self {
                _dir: dir,
                router: router().with_state(state.clone()),
                state,
            }
        }

        fn revision(&self) -> u64 {
            self.state.store().revision()
        }

        async fn call(
            &self,
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
            let response = self.router.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(!text.contains(SECRET), "secret leaked: {text}");
            (
                status,
                serde_json::from_str(&text).unwrap_or(Value::String(text)),
            )
        }
    }

    #[tokio::test]
    async fn list_reports_the_configured_servers() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture.call(Method::GET, "/", None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["servers"].as_array().unwrap().len(), 1);
        assert_eq!(body["servers"][0]["name"], "server1");
        // no backend is registered in this fixture
        assert_eq!(body["servers"][0]["state"], Value::Null);
        // the token reads as a secret, never as its value
        assert_eq!(body["servers"][0]["config"]["token"], REDACTED);
        assert_eq!(
            body["servers"][0]["config"]["url"],
            "ws://10.0.0.100:8080/api"
        );
    }

    #[tokio::test]
    async fn put_creates_a_draft_entry() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture
            .call(
                Method::PUT,
                "/server2",
                Some(json!({
                    "revision": fixture.revision(),
                    "server": {"url": "ws://10.0.0.200:8080", "token": "new-token"},
                })),
            )
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["config"]["z2m"]["server2"]["token"], REDACTED);
        assert_eq!(body["revision"], fixture.revision());

        // the draft holds the real token; the active configuration is untouched
        let draft = fixture.state.store().draft().unwrap().config;
        assert_eq!(
            draft.z2m.servers["server2"].token.as_deref(),
            Some("new-token")
        );
        assert!(!fixture.state.config().z2m.servers.contains_key("server2"));
    }

    #[tokio::test]
    async fn put_keeps_the_existing_token_when_it_is_not_sent() {
        let fixture = Fixture::new().await;

        let (status, _) = fixture
            .call(
                Method::PUT,
                "/server1",
                Some(json!({
                    "revision": fixture.revision(),
                    "server": {"url": "ws://10.0.0.100:9090"},
                })),
            )
            .await;

        assert_eq!(status, StatusCode::OK);
        let draft = fixture.state.store().draft().unwrap().config;
        let server = &draft.z2m.servers["server1"];
        assert_eq!(server.url, url::Url::parse("ws://10.0.0.100:9090").unwrap());
        assert_eq!(server.token.as_deref(), Some(SECRET));
    }

    #[tokio::test]
    async fn delete_removes_the_draft_entry() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture
            .call(
                Method::DELETE,
                "/server1",
                Some(json!({"revision": fixture.revision()})),
            )
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["config"]["z2m"], json!({}));
        assert!(
            fixture
                .state
                .store()
                .draft()
                .unwrap()
                .config
                .z2m
                .servers
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_stale_revision_conflicts() {
        let fixture = Fixture::new().await;
        let server =
            json!({"revision": fixture.revision(), "server": {"url": "ws://10.0.0.200:8080"}});
        fixture
            .call(Method::PUT, "/server2", Some(server.clone()))
            .await;

        let (status, body) = fixture.call(Method::PUT, "/server2", Some(server)).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body["error"].as_str().unwrap().contains("Stale revision"));
    }

    #[tokio::test]
    async fn a_server_name_cannot_contain_an_at_sign() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture
            .call(
                Method::PUT,
                "/with%40at",
                Some(json!({"revision": fixture.revision(), "server": {"url": "ws://10.0.0.200:8080"}})),
            )
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["errors"][0]["path"], "z2m.with@at");
    }

    #[tokio::test]
    async fn get_reports_one_server() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture.call(Method::GET, "/server1", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["name"], "server1");

        let (status, _) = fixture.call(Method::GET, "/missing", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
