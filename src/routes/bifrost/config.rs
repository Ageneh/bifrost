//! Configuration admin API under `/bifrost/config`.
//!
//! Every write names the revision it was based on and fails with
//! `409 Conflict` when that revision is stale. Secrets never leave the
//! process: reads are redacted, and secret fields are write-only.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::future::BoxFuture;
use hyper::StatusCode;
use serde_json::Value;

use bifrost_api::admin::{
    ApplyResponse, ConfigImport, DraftPatch, DraftResponse, DraftWrite, ErrorResponse,
    RestartRequest, RevisionRequest, RevisionResponse,
};

use crate::config::AppConfig;
use crate::config::diff::{self, ApplyAction, ConfigChange};
use crate::config::redact::{merge_patch, merge_secrets, redacted_config};
use crate::config::validate::{self, ValidationError};
use crate::config_store::{ConfigStoreError, Draft};
use crate::error::ApiResult;
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;
use crate::service_registry::{self, AppServiceFactory};

/// Time for the restart response to reach the client before services stop.
const RESTART_GRACE: Duration = Duration::from_millis(500);

const YAML_CONTENT_TYPE: &str = "application/yaml";

/// The authentication settings, which the draft cannot change.
const AUTH_PATH: &str = "bifrost.auth";
const AUTH_POINTER: &str = "/bifrost/auth";

/// Brings running services in line with an applied configuration.
pub type ApplyHook =
    Arc<dyn Fn(AppState, Vec<ApplyAction>) -> BoxFuture<'static, ApiResult<()>> + Send + Sync>;

#[derive(Debug)]
pub enum ConfigApiError {
    Store(ConfigStoreError),
    Invalid(Vec<ValidationError>),
    Conflict(String),
    Internal(String),
}

impl From<ConfigStoreError> for ConfigApiError {
    fn from(err: ConfigStoreError) -> Self {
        match err {
            ConfigStoreError::Invalid(errors) => Self::Invalid(errors),
            err => Self::Store(err),
        }
    }
}

impl From<serde_json::Error> for ConfigApiError {
    fn from(err: serde_json::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<serde_yml::Error> for ConfigApiError {
    fn from(err: serde_yml::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl IntoResponse for ConfigApiError {
    fn into_response(self) -> Response {
        let (status, error, errors) = match self {
            Self::Store(err @ ConfigStoreError::StaleRevision { .. }) => {
                (StatusCode::CONFLICT, err.to_string(), vec![])
            }
            Self::Store(err @ ConfigStoreError::NoDraft) => {
                (StatusCode::NOT_FOUND, err.to_string(), vec![])
            }
            Self::Invalid(errors) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "Configuration is invalid".to_string(),
                errors,
            ),
            Self::Conflict(err) => (StatusCode::CONFLICT, err, vec![]),
            Self::Store(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string(), vec![]),
            Self::Internal(err) => (StatusCode::INTERNAL_SERVER_ERROR, err, vec![]),
        };

        log::warn!("Configuration request failed: {error}");
        (status, Json(ErrorResponse { error, errors })).into_response()
    }
}

pub(crate) type ConfigApiResult<T> = Result<T, ConfigApiError>;

fn draft_response(revision: u64, draft: Option<&Draft>) -> ConfigApiResult<DraftResponse> {
    let Some(draft) = draft else {
        return Ok(DraftResponse {
            revision,
            updated_at: None,
            config: None,
            validation: vec![],
        });
    };

    Ok(DraftResponse {
        revision,
        updated_at: Some(draft.updated_at.to_rfc3339()),
        config: Some(redacted_config(&draft.config)?),
        validation: validate::validate(&draft.config),
    })
}

/// The configuration a draft write starts from: the draft, or the active
/// configuration when there is none.
fn base_config(state: &AppState) -> AppConfig {
    let store = state.store();
    store
        .draft()
        .map_or_else(|| store.active(), |draft| draft.config)
}

/// Resolve write-only secrets against the base, then parse and store.
fn write_draft(
    state: &AppState,
    revision: u64,
    mut document: Value,
) -> ConfigApiResult<DraftResponse> {
    let base = serde_json::to_value(base_config(state))?;
    merge_secrets(&mut document, &base);

    let (config, _findings) =
        validate::validate_value(&document).map_err(ConfigApiError::Invalid)?;

    let store = state.store();

    // a document without the section keeps the active settings; one that
    // names different settings is refused rather than silently dropped
    if document.pointer(AUTH_POINTER).is_some()
        && config.bifrost.auth != store.active().bifrost.auth
    {
        return Err(ConfigApiError::Invalid(vec![ValidationError::error(
            AUTH_PATH,
            "Authentication settings cannot be changed through the draft; use /bifrost/auth",
        )]));
    }

    let draft = store.set_draft(revision, config)?;
    draft_response(store.revision(), Some(&draft))
}

async fn get_draft(State(state): State<AppState>) -> ConfigApiResult<Json<DraftResponse>> {
    let store = state.store();
    Ok(Json(draft_response(
        store.revision(),
        store.draft().as_ref(),
    )?))
}

async fn put_draft(
    State(state): State<AppState>,
    Json(req): Json<DraftWrite>,
) -> ConfigApiResult<Json<DraftResponse>> {
    Ok(Json(write_draft(&state, req.revision, req.config)?))
}

/// Apply `patch` (RFC 7386) to the draft, or to the active configuration
/// when there is none, and store the result as the draft.
///
/// This is what the typed endpoints (the z2m server list, for one) write
/// through, so they inherit the revision check, the secret handling and the
/// validation of a hand-written patch.
pub(crate) fn patch_draft(
    state: &AppState,
    revision: u64,
    patch: &Value,
) -> ConfigApiResult<DraftResponse> {
    let mut document = serde_json::to_value(base_config(state))?;
    merge_patch(&mut document, patch);
    write_draft(state, revision, document)
}

async fn patch_draft_handler(
    State(state): State<AppState>,
    Json(req): Json<DraftPatch>,
) -> ConfigApiResult<Json<DraftResponse>> {
    Ok(Json(patch_draft(&state, req.revision, &req.patch)?))
}

async fn delete_draft(
    State(state): State<AppState>,
    Json(req): Json<RevisionRequest>,
) -> ConfigApiResult<Json<RevisionResponse>> {
    let revision = state.store().discard_draft(req.revision)?;
    Ok(Json(RevisionResponse { revision }))
}

async fn get_diff(State(state): State<AppState>) -> Json<Vec<ConfigChange>> {
    let store = state.store();
    let changes = store
        .draft()
        .map(|draft| diff::diff(&store.active(), &draft.config))
        .unwrap_or_default();
    Json(changes)
}

async fn get_validate(State(state): State<AppState>) -> Json<Vec<ValidationError>> {
    Json(validate::validate(&base_config(&state)))
}

async fn post_validate(Json(document): Json<Value>) -> Json<Vec<ValidationError>> {
    Json(match validate::validate_value(&document) {
        Ok((_config, findings)) => findings,
        Err(errors) => errors,
    })
}

fn apply(state: &AppState, hook: &ApplyHook, revision: u64) -> ConfigApiResult<ApplyResponse> {
    let outcome = state.store().apply(revision)?;

    log::info!(
        "Applied configuration revision {}: {} change(s), {} restart(s) required",
        outcome.revision,
        outcome.changes.len(),
        outcome.plan.restart_required.len()
    );

    // Run after responding: restarting the http listeners from inside a
    // request they serve would cut that request off.
    let actions = outcome.plan.hot.clone();
    let task = hook(state.clone(), actions);
    tokio::spawn(async move {
        if let Err(err) = task.await {
            log::error!("Failed to apply configuration to running services: {err}");
        }
    });

    Ok(ApplyResponse {
        revision: outcome.revision,
        plan: outcome.plan,
        changes: outcome.changes,
    })
}

async fn post_apply(
    State(state): State<AppState>,
    hook: axum::Extension<ApplyHook>,
    Json(req): Json<RevisionRequest>,
) -> ConfigApiResult<Json<ApplyResponse>> {
    Ok(Json(apply(&state, &hook, req.revision)?))
}

async fn post_restart(
    State(state): State<AppState>,
    hook: axum::Extension<ApplyHook>,
    Json(req): Json<RestartRequest>,
) -> ConfigApiResult<Json<Option<ApplyResponse>>> {
    let store = state.store();

    let applied = if store.draft().is_some() {
        let revision = req.revision.ok_or_else(|| {
            ConfigApiError::Conflict(format!(
                "A draft exists; name its revision ({}) to apply it before restarting",
                store.revision()
            ))
        })?;
        Some(apply(&state, &hook, revision)?)
    } else {
        None
    };

    log::warn!("Restart requested through the admin API, shutting down..");
    let mut mgr = state.manager();
    tokio::spawn(async move {
        tokio::time::sleep(RESTART_GRACE).await;
        // main exits with status 0 once the service manager has stopped,
        // and the supervisor starts Bifrost again
        if let Err(err) = mgr.shutdown().await {
            log::error!("Failed to shut down for restart: {err}");
        }
    });

    Ok(Json(applied))
}

async fn get_export(State(state): State<AppState>) -> ConfigApiResult<Response> {
    let document = redacted_config(&state.config())?;
    let yaml = serde_yml::to_string(&document)?;
    Ok(([(CONTENT_TYPE, YAML_CONTENT_TYPE)], yaml).into_response())
}

async fn post_import(
    State(state): State<AppState>,
    Json(req): Json<ConfigImport>,
) -> ConfigApiResult<Json<DraftResponse>> {
    let document: Value = serde_yml::from_str(&req.yaml).map_err(|err| {
        ConfigApiError::Invalid(vec![ValidationError::error("", err.to_string())])
    })?;
    Ok(Json(write_draft(&state, req.revision, document)?))
}

async fn get_config(State(state): State<AppState>) -> ConfigApiResult<Json<Value>> {
    Ok(Json(redacted_config(&state.config())?))
}

/// The admin endpoints, without the active configuration.
pub fn admin_router(hook: ApplyHook) -> Router<AppState> {
    Router::new()
        .route(
            "/draft",
            get(get_draft)
                .put(put_draft)
                .patch(patch_draft_handler)
                .delete(delete_draft),
        )
        .route("/diff", get(get_diff))
        .route("/validate", get(get_validate).post(post_validate))
        .route("/apply", post(post_apply))
        .route("/restart", post(post_restart))
        .route("/export", get(get_export))
        .route("/import", post(post_import))
        .layer(axum::Extension(hook))
}

fn reconcile_hook() -> ApplyHook {
    Arc::new(|state: AppState, actions: Vec<ApplyAction>| {
        Box::pin(async move {
            let factory = AppServiceFactory::new(state.clone());
            let mut mgr = state.manager();
            service_registry::reconcile(&state.store(), &mut mgr, &factory, &actions).await
        })
    })
}

/// The configuration routes. The access guard is applied by the `/bifrost`
/// nest, which covers `GET /bifrost/config` too.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_config))
        .merge(admin_router(reconcile_hook()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use hyper::StatusCode;
    use serde_json::{Value, json};
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use crate::config::diff::{ApplyAction, HTTP_SERVICE};
    use crate::config::redact::REDACTED;
    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::config::{ApplyHook, admin_router, router};
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    const SECRET: &str = "s3cret-token-value";

    struct Fixture {
        _dir: TempDir,
        state: AppState,
        admin: Router,
        applied: Arc<Mutex<Vec<Vec<ApplyAction>>>>,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = TempDir::new();
            let yaml = MINIMAL_CONFIG.replace(
                "url: ws://10.0.0.100:8080",
                &format!("url: ws://10.0.0.100:8080/api?token={SECRET}"),
            );
            let store = ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap();
            let (svm, _future) = ServiceManager::spawn();
            let state = AppState::from_store(Arc::new(store), svm).await.unwrap();

            let applied = Arc::new(Mutex::new(vec![]));
            let recorder = applied.clone();
            let hook: ApplyHook = Arc::new(move |_state, actions| {
                recorder.lock().unwrap().push(actions);
                Box::pin(async { Ok(()) })
            });

            Self {
                _dir: dir,
                admin: admin_router(hook).with_state(state.clone()),
                state,
                applied,
            }
        }

        async fn call(
            &self,
            method: Method,
            uri: &str,
            body: Option<Value>,
        ) -> (StatusCode, Value) {
            call(&self.admin, method, uri, body).await
        }

        async fn put_draft(&self, revision: u64, config: Value) -> (StatusCode, Value) {
            self.call(
                Method::PUT,
                "/draft",
                Some(json!({"revision": revision, "config": config})),
            )
            .await
        }

        async fn active_document(&self) -> Value {
            let (status, draft) = self.call(Method::GET, "/draft", None).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(draft["config"], Value::Null);
            serde_json::to_value(self.state.store().active()).unwrap()
        }
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
            .body(body)
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!text.contains(SECRET), "secret leaked: {text}");
        let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, value)
    }

    #[tokio::test]
    async fn no_draft_initially() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture.call(Method::GET, "/draft", None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["revision"], 0);
        assert_eq!(body["config"], Value::Null);
    }

    #[tokio::test]
    async fn put_draft_persists_without_applying() {
        let fixture = Fixture::new().await;
        let mut config = fixture.active_document().await;
        config["bridge"]["name"] = json!("Draft");

        let (status, body) = fixture.put_draft(0, config).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["revision"], 1);
        assert_eq!(body["config"]["bridge"]["name"], "Draft");
        assert_eq!(fixture.state.config().bridge.name, "Bifrost");
        assert!(fixture.state.store().paths().draft.exists());

        let (_, body) = fixture.call(Method::GET, "/diff", None).await;
        assert_eq!(
            body,
            json!([{"path": "bridge.name", "before": "Bifrost", "after": "Draft", "secret": false}])
        );
    }

    #[tokio::test]
    async fn stale_revision_conflicts() {
        let fixture = Fixture::new().await;
        let config = fixture.active_document().await;
        fixture.put_draft(0, config.clone()).await;

        let (status, body) = fixture.put_draft(0, config).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body["error"].as_str().unwrap().contains("Stale revision"));
    }

    #[tokio::test]
    async fn unparsable_draft_is_rejected_with_paths() {
        let fixture = Fixture::new().await;
        let mut config = fixture.active_document().await;
        config["bridge"]["mac"] = json!("not-a-mac");

        let (status, body) = fixture.put_draft(0, config).await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"][0]["path"], "bridge.mac");
        assert_eq!(fixture.state.store().revision(), 0);
    }

    #[tokio::test]
    async fn patch_changes_one_setting() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture
            .call(
                Method::PATCH,
                "/draft",
                Some(json!({"revision": 0, "patch": {"bridge": {"http_port": 8080}}})),
            )
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["config"]["bridge"]["http_port"], 8080);
        assert_eq!(body["config"]["bridge"]["name"], "Bifrost");
    }

    #[tokio::test]
    async fn redacted_round_trip_keeps_secret() {
        let fixture = Fixture::new().await;

        // the guard sits on the /bifrost nest, so this router serves GET /
        let (_, active) = call(
            &router().with_state(fixture.state.clone()),
            Method::GET,
            "/",
            None,
        )
        .await;
        let url = active["z2m"]["server1"]["url"].as_str().unwrap();
        assert!(url.contains(REDACTED), "{url}");

        let mut config = active.clone();
        config["bridge"]["name"] = json!("Renamed");
        let (status, _) = fixture.put_draft(0, config).await;
        assert_eq!(status, StatusCode::OK);

        let draft = fixture.state.store().draft().unwrap().config;
        assert!(draft.z2m.servers["server1"].url.as_str().contains(SECRET));
    }

    #[tokio::test]
    async fn apply_writes_backup_and_returns_plan() {
        let fixture = Fixture::new().await;
        fixture
            .call(
                Method::PATCH,
                "/draft",
                Some(
                    json!({"revision": 0, "patch": {"bridge": {"http_port": 8080, "name": "New"}}}),
                ),
            )
            .await;

        let (status, body) = fixture
            .call(Method::POST, "/apply", Some(json!({"revision": 1})))
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["revision"], 2);
        assert_eq!(
            body["plan"]["hot"],
            json!([
                {"action": "restart_service", "service": HTTP_SERVICE},
                {"action": "update", "path": "bridge.name"},
            ])
        );
        assert_eq!(body["plan"]["restart_required"], json!([]));

        let store = fixture.state.store();
        assert_eq!(store.active().bridge.http_port, 8080);
        assert!(store.draft().is_none());
        assert!(store.paths().backup.exists());
        assert_eq!(fixture.applied.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn apply_reports_restart_required() {
        let fixture = Fixture::new().await;
        fixture
            .call(
                Method::PATCH,
                "/draft",
                Some(json!({"revision": 0, "patch": {"bridge": {"mac": "00:11:22:33:44:66"}}})),
            )
            .await;

        let (_, body) = fixture
            .call(Method::POST, "/apply", Some(json!({"revision": 1})))
            .await;

        assert_eq!(body["plan"]["hot"], json!([]));
        assert_eq!(body["plan"]["restart_required"][0]["path"], "bridge.mac");
    }

    #[tokio::test]
    async fn apply_refuses_invalid_draft() {
        let fixture = Fixture::new().await;
        fixture
            .call(
                Method::PATCH,
                "/draft",
                Some(json!({"revision": 0, "patch": {"bridge": {"https_port": 80}}})),
            )
            .await;

        let (status, body) = fixture
            .call(Method::POST, "/apply", Some(json!({"revision": 1})))
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"][0]["path"], "bridge.https_port");
        assert!(fixture.state.store().draft().is_some());
        assert!(fixture.applied.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn apply_without_draft_is_not_found() {
        let fixture = Fixture::new().await;

        let (status, _) = fixture
            .call(Method::POST, "/apply", Some(json!({"revision": 0})))
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_discards_draft() {
        let fixture = Fixture::new().await;
        fixture.put_draft(0, fixture.active_document().await).await;

        let (status, body) = fixture
            .call(Method::DELETE, "/draft", Some(json!({"revision": 1})))
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["revision"], 2);
        assert!(fixture.state.store().draft().is_none());
    }

    #[tokio::test]
    async fn validate_reports_findings() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture.call(Method::GET, "/validate", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!([]));

        let mut config = fixture.active_document().await;
        config["bridge"]["entm_port"] = json!(80);
        let (status, body) = fixture.call(Method::POST, "/validate", Some(config)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body[0]["path"], "bridge.entm_port");
        assert_eq!(body[0]["severity"], "error");
    }

    #[tokio::test]
    async fn export_then_import_creates_draft() {
        let fixture = Fixture::new().await;

        let (status, exported) = fixture.call(Method::GET, "/export", None).await;
        assert_eq!(status, StatusCode::OK);
        let yaml = exported
            .as_str()
            .unwrap()
            .replace("name: Bifrost", "name: Imported");

        let (status, body) = fixture
            .call(
                Method::POST,
                "/import",
                Some(json!({"revision": 0, "yaml": yaml})),
            )
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["config"]["bridge"]["name"], "Imported");
        let draft = fixture.state.store().draft().unwrap().config;
        assert!(draft.z2m.servers["server1"].url.as_str().contains(SECRET));
    }

    #[tokio::test]
    async fn restart_with_draft_needs_revision() {
        let fixture = Fixture::new().await;
        fixture.put_draft(0, fixture.active_document().await).await;

        let (status, _) = fixture
            .call(Method::POST, "/restart", Some(json!({})))
            .await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert!(fixture.state.store().draft().is_some());
    }

    #[tokio::test]
    async fn restart_applies_draft_and_shuts_down() {
        let fixture = Fixture::new().await;
        let mut config = fixture.active_document().await;
        config["bridge"]["name"] = json!("After restart");
        fixture.put_draft(0, config).await;

        let (status, body) = fixture
            .call(Method::POST, "/restart", Some(json!({"revision": 1})))
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["revision"], 2);
        assert_eq!(fixture.state.config().bridge.name, "After restart");
    }

    #[tokio::test]
    async fn draft_refuses_auth_changes_and_keeps_auth_on_round_trip() {
        let fixture = Fixture::new().await;
        fixture
            .state
            .store()
            .update_auth(|auth| {
                auth.password_hash = Some("$scrypt$owner-hash".into());
                true
            })
            .unwrap();

        // the redacted document round-trips with the hash intact
        let (_, active) = call(
            &router().with_state(fixture.state.clone()),
            Method::GET,
            "/",
            None,
        )
        .await;
        assert_eq!(active["bifrost"]["auth"]["password_hash"], REDACTED);
        let (status, _) = fixture.put_draft(0, active.clone()).await;
        assert_eq!(status, StatusCode::OK);

        // a document without the section keeps the active settings
        let mut without = active.clone();
        without["bifrost"].as_object_mut().unwrap().remove("auth");
        let (status, _) = fixture.put_draft(1, without).await;
        assert_eq!(status, StatusCode::OK);

        for patch in [
            json!({"bifrost": {"auth": {"trusted_cidrs": ["0.0.0.0/0"]}}}),
            json!({"bifrost": {"auth": {"password_hash": "$scrypt$attacker"}}}),
            json!({"bifrost": {"auth": {"password_hash": ""}}}),
            json!({"bifrost": {"auth": {"trust_ingress": false}}}),
        ] {
            let (status, body) = fixture
                .call(
                    Method::PATCH,
                    "/draft",
                    Some(json!({"revision": 2, "patch": patch})),
                )
                .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{patch}");
            assert_eq!(body["errors"][0]["path"], "bifrost.auth", "{patch}");
        }

        let (status, _) = fixture
            .call(Method::POST, "/apply", Some(json!({"revision": 2})))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            fixture.state.config().bifrost.auth.password_hash.as_deref(),
            Some("$scrypt$owner-hash")
        );
    }

    #[tokio::test]
    async fn token_field_is_write_only() {
        let fixture = Fixture::new().await;
        let unguarded = router().with_state(fixture.state.clone());

        // the first apply moves the url token into the token field
        fixture.put_draft(0, fixture.active_document().await).await;
        fixture
            .call(Method::POST, "/apply", Some(json!({"revision": 1})))
            .await;
        let (_, active) = call(&unguarded, Method::GET, "/", None).await;
        assert_eq!(active["z2m"]["server1"]["token"], REDACTED);
        assert_eq!(active["z2m"]["server1"]["url"], "ws://10.0.0.100:8080/api");

        // null keeps the token
        let mut config = active.clone();
        config["z2m"]["server1"]["token"] = Value::Null;
        fixture.put_draft(2, config).await;
        let draft = fixture.state.store().draft().unwrap().config;
        assert_eq!(draft.z2m.servers["server1"].token.as_deref(), Some(SECRET));

        // "" clears it
        let mut config = active;
        config["z2m"]["server1"]["token"] = json!("");
        fixture.put_draft(3, config).await;
        let draft = fixture.state.store().draft().unwrap().config;
        assert_eq!(draft.z2m.servers["server1"].token, None);

        let (_, changes) = fixture.call(Method::GET, "/diff", None).await;
        assert_eq!(
            changes,
            json!([{"path": "z2m.server1.token", "before": REDACTED, "after": null, "secret": true}])
        );
    }
}
