//! Setup mode: what Bifrost serves while its configuration file is missing
//! or cannot be read.
//!
//! Only this module knows a configuration can be absent; everything else
//! keeps working with a complete [`AppConfig`]. Setup mode serves the admin
//! login and a configuration draft, nothing that needs a running bridge
//! (no services, logs, state or websocket). Every route but the login sits
//! behind a session, and the first session needs the setup code printed to
//! the console. Applying the draft writes the configuration file, and the
//! caller then starts the bridge in the same process.

use std::fs;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;

use bifrost_api::admin::{
    ApplyResponse, ConfigImport, DraftPatch, DraftResponse, DraftWrite, ErrorResponse,
    RevisionRequest, SetupStatus,
};

use crate::auth::{Auth, HashCost, SystemClock};
use crate::config::redact::{merge_patch, merge_secrets, redacted};
use crate::config::validate::{self, ValidationError};
use crate::config::{AppConfig, AuthConfig, DEFAULT_HTTP_PORT};
use crate::config_store::{ConfigPaths, ConfigStoreError, write_atomic};
use crate::routes::bifrost::auth::{self, AuthHost};
use crate::routes::bifrost::{check_access, not_found};
use crate::routes::extractor::Json;

/// Time for the apply response to reach the client before the setup
/// listener closes.
const HANDOVER_GRACE: Duration = Duration::from_millis(500);

/// A configuration file that could not be read is kept under this suffix
/// when setup replaces it; `.bak` keeps the last applied configuration.
const BROKEN_SUFFIX: &str = "broken";

/// The authentication settings, which the draft cannot change.
const AUTH_PATH: &str = "bifrost.auth";
const AUTH_POINTER: &str = "/bifrost/auth";

/// Any address outside the local networks; connecting a UDP socket to it
/// sends nothing, but picks the interface of the default route.
const ROUTE_PROBE: &str = "192.0.2.1:9";

/// The setup draft. Unlike the running bridge's draft, it may be incomplete:
/// a wizard saves every step. The file uses the same fields, so a draft
/// left from before the configuration broke is picked up.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SetupDraft {
    config: Value,
    revision: u64,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct DraftSlot {
    draft: Option<SetupDraft>,
    revision: u64,
}

struct Inner {
    paths: ConfigPaths,
    reason: String,
    listen: IpAddr,
    auth: Arc<Auth>,
    auth_config: Mutex<AuthConfig>,
    draft: Mutex<DraftSlot>,
    configured: watch::Sender<bool>,
}

/// Shared state of setup mode.
#[derive(Clone)]
pub struct SetupState(Arc<Inner>);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SetupState {
    /// Setup mode for `config_file`, which could not be used because of
    /// `reason`, answering on `listen`.
    #[must_use]
    pub fn new(config_file: &Utf8Path, reason: String, listen: IpAddr, auth: Arc<Auth>) -> Self {
        let paths = ConfigPaths::with_defaults(config_file);

        let draft = fs::read_to_string(&paths.draft).ok().and_then(|text| {
            serde_yml::from_str::<SetupDraft>(&text)
                .inspect_err(|err| {
                    log::warn!(
                        "Ignoring unreadable draft configuration {}: {err}",
                        paths.draft
                    );
                })
                .ok()
        });
        let revision = draft.as_ref().map_or(0, |draft| draft.revision);

        Self(Arc::new(Inner {
            paths,
            reason,
            listen,
            auth,
            auth_config: Mutex::new(AuthConfig::default()),
            draft: Mutex::new(DraftSlot { draft, revision }),
            configured: watch::channel(false).0,
        }))
    }

    #[must_use]
    pub fn paths(&self) -> &ConfigPaths {
        &self.0.paths
    }

    /// Resolves once a configuration has been applied.
    pub async fn configured(&self) {
        let mut receiver = self.0.configured.subscribe();
        let _ = receiver.wait_for(|done| *done).await;
    }

    /// Setup mode answers to the address it listens on only, or to any
    /// address literal when it listens on all of them. A DNS name, which an
    /// attacker could rebind to this host, is refused.
    fn host_allowed(&self, host: &str) -> bool {
        let Ok(addr) = host.parse::<IpAddr>() else {
            return false;
        };
        self.0.listen.is_unspecified() || addr == self.0.listen
    }
}

impl AuthHost for SetupState {
    fn auth(&self) -> Arc<Auth> {
        self.0.auth.clone()
    }

    fn auth_config(&self) -> AuthConfig {
        lock(&self.0.auth_config).clone()
    }

    // the hash is written with the configuration on apply
    fn set_password_hash(&self, hash: String, only_first: bool) -> Result<bool, ConfigStoreError> {
        let mut config = lock(&self.0.auth_config);
        if only_first && config.password_hash.is_some() {
            return Ok(false);
        }
        config.password_hash = Some(hash);
        drop(config);
        Ok(true)
    }
}

#[derive(Debug)]
pub enum SetupApiError {
    Stale { expected: u64, current: u64 },
    NoDraft,
    Invalid(Vec<ValidationError>),
    Internal(String),
}

impl From<io::Error> for SetupApiError {
    fn from(err: io::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<serde_yml::Error> for SetupApiError {
    fn from(err: serde_yml::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl IntoResponse for SetupApiError {
    fn into_response(self) -> Response {
        let (status, error, errors) = match self {
            Self::Stale { expected, current } => (
                StatusCode::CONFLICT,
                format!("Stale revision {expected}, current revision is {current}"),
                vec![],
            ),
            Self::NoDraft => (
                StatusCode::NOT_FOUND,
                "No draft configuration exists".to_string(),
                vec![],
            ),
            Self::Invalid(errors) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "Configuration is invalid".to_string(),
                errors,
            ),
            Self::Internal(err) => (StatusCode::INTERNAL_SERVER_ERROR, err, vec![]),
        };

        log::warn!("Setup request failed: {error}");
        (status, Json(ErrorResponse { error, errors })).into_response()
    }
}

type SetupApiResult<T> = Result<T, SetupApiError>;

/// Findings for a draft document: parse errors, or validation findings.
fn findings(document: &Value) -> Vec<ValidationError> {
    match validate::validate_value(document) {
        Ok((_config, findings)) => findings,
        Err(errors) => errors,
    }
}

fn draft_response(slot: &DraftSlot) -> DraftResponse {
    let Some(draft) = &slot.draft else {
        return DraftResponse {
            revision: slot.revision,
            updated_at: None,
            config: None,
            validation: vec![],
        };
    };

    DraftResponse {
        revision: slot.revision,
        updated_at: Some(draft.updated_at.to_rfc3339()),
        config: Some(redacted(&draft.config)),
        validation: findings(&draft.config),
    }
}

const fn check_revision(slot: &DraftSlot, expected: u64) -> SetupApiResult<()> {
    if expected == slot.revision {
        Ok(())
    } else {
        Err(SetupApiError::Stale {
            expected,
            current: slot.revision,
        })
    }
}

/// Store `document` as the draft. Secrets resolve against the previous
/// draft, as in the running bridge's draft API.
// the draft lock is held across the file write, so revisions stay in order
#[allow(clippy::significant_drop_tightening)]
fn write_draft(
    state: &SetupState,
    expected: u64,
    mut document: Value,
) -> SetupApiResult<DraftResponse> {
    let mut slot = lock(&state.0.draft);
    check_revision(&slot, expected)?;

    if let Some(previous) = &slot.draft {
        merge_secrets(&mut document, &previous.config);
    }

    if document.pointer(AUTH_POINTER).is_some() {
        return Err(SetupApiError::Invalid(vec![ValidationError::error(
            AUTH_PATH,
            "Authentication settings cannot be changed through the draft; use /bifrost/auth",
        )]));
    }

    let draft = SetupDraft {
        config: document,
        revision: slot.revision + 1,
        updated_at: Utc::now(),
    };
    write_atomic(
        &state.0.paths.draft,
        serde_yml::to_string(&draft)?.as_bytes(),
    )?;
    slot.revision = draft.revision;
    slot.draft = Some(draft);

    Ok(draft_response(&slot))
}

fn current_document(state: &SetupState) -> Value {
    lock(&state.0.draft).draft.as_ref().map_or_else(
        || Value::Object(serde_json::Map::new()),
        |draft| draft.config.clone(),
    )
}

async fn get_status(State(state): State<SetupState>) -> Json<SetupStatus> {
    Json(SetupStatus {
        config_file: state.0.paths.config.to_string(),
        reason: state.0.reason.clone(),
        backup_available: state.0.paths.backup.is_file(),
    })
}

/// Load the last applied configuration (`config.yaml.bak`) into the draft,
/// secrets included; apply it to restore it.
async fn post_restore(
    State(state): State<SetupState>,
    Json(req): Json<RevisionRequest>,
) -> SetupApiResult<Json<DraftResponse>> {
    let text = fs::read_to_string(&state.0.paths.backup)?;
    let mut document: Value = serde_yml::from_str(&text)
        .map_err(|err| SetupApiError::Invalid(vec![ValidationError::error("", err.to_string())]))?;
    if let Some(bifrost) = document.get_mut("bifrost").and_then(Value::as_object_mut) {
        // the password chosen in setup replaces the backed-up one
        bifrost.remove("auth");
    }
    Ok(Json(write_draft(&state, req.revision, document)?))
}

async fn get_draft(State(state): State<SetupState>) -> Json<DraftResponse> {
    Json(draft_response(&lock(&state.0.draft)))
}

async fn put_draft(
    State(state): State<SetupState>,
    Json(req): Json<DraftWrite>,
) -> SetupApiResult<Json<DraftResponse>> {
    Ok(Json(write_draft(&state, req.revision, req.config)?))
}

async fn patch_draft(
    State(state): State<SetupState>,
    Json(req): Json<DraftPatch>,
) -> SetupApiResult<Json<DraftResponse>> {
    let mut document = current_document(&state);
    merge_patch(&mut document, &req.patch);
    Ok(Json(write_draft(&state, req.revision, document)?))
}

#[allow(clippy::significant_drop_tightening)]
async fn delete_draft(
    State(state): State<SetupState>,
    Json(req): Json<RevisionRequest>,
) -> SetupApiResult<Json<DraftResponse>> {
    let mut slot = lock(&state.0.draft);
    check_revision(&slot, req.revision)?;
    if slot.draft.take().is_none() {
        return Err(SetupApiError::NoDraft);
    }
    match fs::remove_file(&state.0.paths.draft) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.into()),
        _ => {}
    }
    slot.revision += 1;
    Ok(Json(draft_response(&slot)))
}

async fn get_validate(State(state): State<SetupState>) -> Json<Vec<ValidationError>> {
    Json(findings(&current_document(&state)))
}

async fn post_validate(Json(document): Json<Value>) -> Json<Vec<ValidationError>> {
    Json(findings(&document))
}

async fn post_import(
    State(state): State<SetupState>,
    Json(req): Json<ConfigImport>,
) -> SetupApiResult<Json<DraftResponse>> {
    let document: Value = serde_yml::from_str(&req.yaml)
        .map_err(|err| SetupApiError::Invalid(vec![ValidationError::error("", err.to_string())]))?;
    Ok(Json(write_draft(&state, req.revision, document)?))
}

/// Keep the unreadable file beside the new one, so nothing is lost.
fn keep_broken_file(config_file: &Utf8Path) -> io::Result<()> {
    match fs::read(config_file) {
        Ok(data) => write_atomic(
            &Utf8PathBuf::from(format!("{config_file}.{BROKEN_SUFFIX}")),
            &data,
        ),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Write the draft as the configuration file, with the password chosen in
/// setup, and hand over to the bridge.
#[allow(clippy::significant_drop_tightening)]
async fn post_apply(
    State(state): State<SetupState>,
    Json(req): Json<RevisionRequest>,
) -> SetupApiResult<Json<ApplyResponse>> {
    let mut slot = lock(&state.0.draft);
    check_revision(&slot, req.revision)?;
    let document = slot
        .draft
        .as_ref()
        .map(|draft| draft.config.clone())
        .ok_or(SetupApiError::NoDraft)?;

    let (mut config, findings): (AppConfig, _) =
        validate::validate_value(&document).map_err(SetupApiError::Invalid)?;
    let errors: Vec<ValidationError> = findings
        .into_iter()
        .filter(ValidationError::is_error)
        .collect();
    if !errors.is_empty() {
        return Err(SetupApiError::Invalid(errors));
    }

    config.bifrost.auth = state.auth_config();
    config.migrate_url_tokens();

    let config_file = &state.0.paths.config;
    keep_broken_file(config_file)?;
    write_atomic(config_file, serde_yml::to_string(&config)?.as_bytes())?;
    if let Err(err) = fs::remove_file(&state.0.paths.draft) {
        if err.kind() != io::ErrorKind::NotFound {
            log::warn!(
                "Failed to remove applied draft {}: {err}",
                state.0.paths.draft
            );
        }
    }
    slot.draft = None;
    slot.revision += 1;

    log::info!("Setup wrote {config_file}; starting the bridge");
    state.0.configured.send_replace(true);

    Ok(Json(ApplyResponse {
        revision: slot.revision,
        plan: bifrost_api::admin::ApplyPlan::default(),
        changes: vec![],
        failed: vec![],
    }))
}

async fn require_setup_admin(
    State(state): State<SetupState>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let host_allowed = |host: &str| state.host_allowed(host);

    match check_access(
        &mut parts,
        &state.0.auth,
        &state.auth_config(),
        &host_allowed,
    ) {
        None => next.run(Request::from_parts(parts, body)).await,
        Some(refusal) => refusal,
    }
}

async fn setup_page() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Bifrost is in setup mode: its configuration file is missing or cannot be read.\n",
    )
        .into_response()
}

/// Every route setup mode serves.
pub fn router(state: SetupState) -> Router<()> {
    let bifrost = Router::new()
        .nest("/auth", auth::session_router())
        .route("/setup/status", get(get_status))
        .route("/setup/restore", post(post_restore))
        .route(
            "/config/draft",
            get(get_draft)
                .put(put_draft)
                .patch(patch_draft)
                .delete(delete_draft),
        )
        .route("/config/validate", get(get_validate).post(post_validate))
        .route("/config/import", post(post_import))
        .route("/config/apply", post(post_apply))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_setup_admin,
        ));

    Router::new()
        .nest("/bifrost", bifrost)
        .fallback(setup_page)
        .with_state(state)
}

/// The address setup mode listens on when none is given: the interface of
/// the default route, on the default HTTP port.
pub fn default_listen() -> io::Result<SocketAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0))?;
    socket.connect(ROUTE_PROBE)?;
    Ok(SocketAddr::new(
        socket.local_addr()?.ip(),
        DEFAULT_HTTP_PORT,
    ))
}

/// How setup mode ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupOutcome {
    /// A configuration was written; start the bridge.
    Configured,
    /// Bifrost was asked to exit.
    Interrupted,
}

/// Serve setup mode on `listen` until a configuration is applied or
/// Bifrost is asked to exit.
pub async fn run(
    config_file: &Utf8Path,
    reason: String,
    listen: SocketAddr,
) -> io::Result<SetupOutcome> {
    let paths = ConfigPaths::with_defaults(config_file);
    let auth = Arc::new(Auth::load(
        &paths.sessions,
        Arc::new(SystemClock),
        HashCost::DEFAULT,
    ));
    let state = SetupState::new(config_file, reason, listen.ip(), auth);

    let listener = std::net::TcpListener::bind(listen)?;
    listener.set_nonblocking(true)?;

    let code = state.auth().issue_setup_code();
    // stdout only, never the log: see main's announce_setup
    println!(
        "Bifrost is in setup mode. Open http://{listen}/ in a browser and enter this setup code: {code}"
    );
    log::warn!(
        "Configuration {config_file} is missing or cannot be read; serving setup mode on {listen}. \
         A one-time setup code was printed to standard output."
    );

    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = serve(state, listener) => result,
        _ = tokio::signal::ctrl_c() => Ok(SetupOutcome::Interrupted),
        _ = terminate.recv() => Ok(SetupOutcome::Interrupted),
    }
}

/// Serve setup mode on `listener` until a configuration is applied, then
/// close the listener so the bridge can bind its own.
pub async fn serve(state: SetupState, listener: std::net::TcpListener) -> io::Result<SetupOutcome> {
    let handle = axum_server::Handle::new();
    let server = axum_server::from_tcp(listener)
        .handle(handle.clone())
        .serve(router(state.clone()).into_make_service_with_connect_info::<SocketAddr>());
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => {
            result?;
            Err(io::Error::other("the setup listener stopped"))
        }
        () = state.configured() => {
            tokio::time::sleep(HANDOVER_GRACE).await;
            handle.graceful_shutdown(Some(Duration::from_secs(1)));
            server.await?;
            Ok(SetupOutcome::Configured)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::ConnectInfo;
    use axum::http::{Method, Request};
    use hyper::StatusCode;
    use serde_json::{Value, json};
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use crate::auth::{Auth, Expiry};
    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::auth::AuthHost;
    use crate::routes::bifrost::setup::{SetupOutcome, SetupState, router, serve};
    use crate::routes::bifrost::tests::{concrete, literal_arguments, routes_dir};
    use crate::server::appstate::AppState;
    use crate::testutil::{
        FAST_HASH, MINIMAL_CONFIG, ManualClock, TempDir, capture_logs, captured_logs,
    };

    const LISTEN: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 12, 0, 20));
    const HOST: &str = "10.12.0.20";
    const ORIGIN: &str = "http://10.12.0.20";
    const OWNER: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 12, 0, 50));

    const PASSWORD: &str = "w2-setup-password-4410";
    const Z2M_TOKEN: &str = "w2-setup-z2m-token-8803";

    /// Routes that answer without a session.
    const PUBLIC_ROUTES: &[(&str, &str)] = &[
        ("GET", "/bifrost/auth/session"),
        ("POST", "/bifrost/auth/login"),
        ("POST", "/bifrost/auth/setup"),
    ];

    /// [`MINIMAL_CONFIG`] as a document, with a z2m token.
    fn minimal_document() -> Value {
        let mut document: Value = serde_yml::from_str(MINIMAL_CONFIG).unwrap();
        document["z2m"]["server1"]["token"] = json!(Z2M_TOKEN);
        document
    }

    struct Fixture {
        dir: TempDir,
        state: SetupState,
        router: Router,
        code: String,
        responses: std::sync::Mutex<Vec<String>>,
    }

    struct Session {
        cookie: String,
        csrf: String,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_file(None)
        }

        fn with_file(contents: Option<&str>) -> Self {
            capture_logs();
            let dir = TempDir::new();
            let config_file = dir.path().join("config.yaml");
            if let Some(contents) = contents {
                fs::write(&config_file, contents).unwrap();
            }
            let auth = Arc::new(Auth::load(
                &dir.path().join("sessions.yaml"),
                ManualClock::new(),
                FAST_HASH,
            ));
            let state = SetupState::new(&config_file, "missing".into(), LISTEN, auth);
            let code = state.auth().issue_setup_code();

            Self {
                router: router(state.clone()),
                dir,
                state,
                code,
                responses: std::sync::Mutex::new(vec![]),
            }
        }

        async fn send(
            &self,
            method: Method,
            uri: &str,
            session: Option<&Session>,
            body: Option<Value>,
            host: &str,
        ) -> (StatusCode, Value) {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header("host", host)
                .header("origin", ORIGIN)
                .header("content-type", "application/json");
            if let Some(session) = session {
                request = request
                    .header("cookie", &session.cookie)
                    .header("x-bifrost-csrf", &session.csrf);
            }
            let body = body.unwrap_or_else(|| json!({}));
            let mut request = request.body(Body::from(body.to_string())).unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::new(OWNER, 50000)));

            let response = self.router.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let cookie = response
                .headers()
                .get("set-cookie")
                .map(|value| value.to_str().unwrap().to_string());
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            self.responses.lock().unwrap().push(text.clone());
            let mut value = serde_json::from_str(&text).unwrap_or(Value::String(text));
            if let (Some(cookie), Value::Object(map)) = (cookie, &mut value) {
                map.insert("__cookie".into(), json!(cookie));
            }
            (status, value)
        }

        async fn call(
            &self,
            method: Method,
            uri: &str,
            session: Option<&Session>,
            body: Option<Value>,
        ) -> (StatusCode, Value) {
            self.send(method, uri, session, body, HOST).await
        }

        async fn set_up(&self) -> Session {
            let (status, body) = self
                .call(
                    Method::POST,
                    "/bifrost/auth/setup",
                    None,
                    Some(json!({"setup_code": self.code, "password": PASSWORD})),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let cookie = body["__cookie"].as_str().unwrap();
            Session {
                cookie: cookie.split(';').next().unwrap().to_string(),
                csrf: body["csrf"].as_str().unwrap().to_string(),
            }
        }

        fn assert_no_secret_leaked(&self, session: &Session) {
            let token = session.cookie.split_once('=').unwrap().1;
            let hash = self.state.auth_config().password_hash.unwrap_or_default();
            let logs = captured_logs();
            let responses = self.responses.lock().unwrap().join("\n");
            for secret in [PASSWORD, Z2M_TOKEN, &self.code, token, &hash] {
                assert!(!logs.contains(secret), "secret in logs");
                assert!(!responses.contains(secret), "secret in a response");
            }
        }
    }

    /// Every route of setup mode, read from this file's source.
    fn scan_setup_routes() -> Vec<String> {
        let source = fs::read_to_string(routes_dir().join("bifrost/setup.rs")).unwrap();
        // the routes, not these tests
        let source = &source[..source.find("#[cfg(test)]").unwrap()];
        let auth = fs::read_to_string(routes_dir().join("bifrost/auth.rs")).unwrap();
        // the auth routes setup mode mounts are session_router's
        let session_router = &auth[auth.find("pub fn session_router").unwrap()..];
        let session_router = &session_router[..session_router.find("\n}\n").unwrap()];

        let mut paths: Vec<String> = literal_arguments(source, ".route(")
            .into_iter()
            .map(|path| format!("/bifrost{path}"))
            .collect();
        paths.extend(
            literal_arguments(session_router, ".route(")
                .into_iter()
                .map(|path| format!("/bifrost/auth{path}")),
        );
        paths.sort();
        paths.dedup();
        paths
    }

    #[tokio::test]
    async fn every_setup_route_is_guarded() {
        let fixture = Fixture::new();
        let paths = scan_setup_routes();
        for expected in [
            "/bifrost/config/apply",
            "/bifrost/setup/status",
            "/bifrost/auth/setup",
        ] {
            assert!(paths.iter().any(|path| path == expected), "{paths:?}");
        }

        for path in paths {
            for method in [
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ] {
                if PUBLIC_ROUTES.contains(&(method.as_str(), path.as_str())) {
                    continue;
                }
                let (status, body) = fixture
                    .call(method.clone(), &concrete(&path), None, None)
                    .await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}: {body}");
            }
        }
    }

    #[tokio::test]
    async fn setup_mode_serves_nothing_that_needs_a_running_bridge() {
        let fixture = Fixture::new();
        let session = fixture.set_up().await;

        for uri in [
            "/bifrost/config",
            "/bifrost/config/diff",
            "/bifrost/config/export",
            "/bifrost/config/restart",
            "/bifrost/service",
            "/bifrost/z2m/x",
            "/bifrost/ws",
            "/bifrost/logs",
            "/bifrost/state/summary",
            "/bifrost/state/reset",
            "/bifrost/auth/password",
        ] {
            let (status, _) = fixture.call(Method::GET, uri, Some(&session), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }

        let (status, _) = fixture.call(Method::GET, "/api/config", None, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn setup_refuses_dns_names() {
        let fixture = Fixture::new();

        for host in ["evil.example", "bifrost.local"] {
            let (status, _) = fixture
                .send(Method::GET, "/bifrost/auth/session", None, None, host)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{host}");
        }
    }

    #[tokio::test]
    async fn setup_needs_the_code() {
        let fixture = Fixture::new();

        let (status, _) = fixture
            .call(
                Method::POST,
                "/bifrost/auth/setup",
                None,
                Some(json!({"setup_code": "AAAA-AAAA-AAAA", "password": PASSWORD})),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = fixture
            .call(Method::GET, "/bifrost/auth/session", None, None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["setup_required"], true);
        assert_eq!(body["authenticated"], false);
    }

    #[tokio::test]
    async fn incomplete_draft_is_kept_but_not_applied() {
        let fixture = Fixture::new();
        let session = fixture.set_up().await;

        let (status, body) = fixture
            .call(
                Method::PATCH,
                "/bifrost/config/draft",
                Some(&session),
                Some(json!({"revision": 0, "patch": {"bridge": {"name": "Wizard"}}})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["config"]["bridge"]["name"], "Wizard");
        assert_ne!(body["validation"], json!([]));

        let (status, _) = fixture
            .call(
                Method::POST,
                "/bifrost/config/apply",
                Some(&session),
                Some(json!({"revision": 1})),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!fixture.state.paths().config.exists());

        // a stale revision conflicts
        let (status, _) = fixture
            .call(
                Method::PATCH,
                "/bifrost/config/draft",
                Some(&session),
                Some(json!({"revision": 0, "patch": {}})),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn draft_cannot_set_auth() {
        let fixture = Fixture::new();
        let session = fixture.set_up().await;
        let mut document = minimal_document();
        document["bifrost"] = json!({"auth": {"trusted_cidrs": ["0.0.0.0/0"]}});

        let (status, body) = fixture
            .call(
                Method::PUT,
                "/bifrost/config/draft",
                Some(&session),
                Some(json!({"revision": 0, "config": document})),
            )
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"][0]["path"], "bifrost.auth");
    }

    #[tokio::test]
    async fn apply_writes_the_configuration_and_keeps_the_login() {
        let fixture = Fixture::new();
        let session = fixture.set_up().await;

        let (status, body) = fixture
            .call(
                Method::PUT,
                "/bifrost/config/draft",
                Some(&session),
                Some(json!({"revision": 0, "config": minimal_document()})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["config"]["z2m"]["server1"]["token"], "<<REDACTED>>");
        assert_eq!(body["validation"], json!([]));

        // a redacted round trip keeps the token
        let (status, _) = fixture
            .call(
                Method::PUT,
                "/bifrost/config/draft",
                Some(&session),
                Some(json!({"revision": 1, "config": body["config"]})),
            )
            .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = fixture
            .call(
                Method::POST,
                "/bifrost/config/apply",
                Some(&session),
                Some(json!({"revision": 2})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        fixture.state.configured().await;
        assert!(!fixture.state.paths().draft.exists());

        // the bridge starts from the written file, password included, and
        // the setup session is still logged in
        let store = ConfigStore::load(&fixture.state.paths().config).unwrap();
        let config = store.active();
        assert_eq!(
            config.z2m.servers["server1"].token.as_deref(),
            Some(Z2M_TOKEN)
        );
        let hash = config.bifrost.auth.password_hash.clone().unwrap();
        assert!(crate::auth::verify_password(PASSWORD, &hash));

        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();
        let token = session.cookie.split_once('=').unwrap().1;
        let expiry = Expiry::from_config(&state.config().bifrost.auth);
        assert!(state.auth().session(token, OWNER, expiry).is_some());

        fixture.assert_no_secret_leaked(&session);
    }

    #[tokio::test]
    async fn broken_file_is_kept_and_backup_restores() {
        let fixture = Fixture::with_file(Some("bridge: [this is not: valid"));
        let backup = MINIMAL_CONFIG.replace("name: Bifrost", "name: From backup");
        let _ = fixture.dir.write("config.yaml.bak", &backup);
        let session = fixture.set_up().await;

        let (status, body) = fixture
            .call(Method::GET, "/bifrost/setup/status", Some(&session), None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["backup_available"], true);

        let (status, body) = fixture
            .call(
                Method::POST,
                "/bifrost/setup/restore",
                Some(&session),
                Some(json!({"revision": 0})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["config"]["bridge"]["name"], "From backup");

        let (status, _) = fixture
            .call(
                Method::POST,
                "/bifrost/config/apply",
                Some(&session),
                Some(json!({"revision": 1})),
            )
            .await;
        assert_eq!(status, StatusCode::OK);

        let broken = fixture.dir.path().join("config.yaml.broken");
        assert_eq!(
            fs::read_to_string(broken).unwrap(),
            "bridge: [this is not: valid"
        );
        assert_eq!(
            fs::read_to_string(fixture.dir.path().join("config.yaml.bak")).unwrap(),
            backup
        );
        let store = ConfigStore::load(&fixture.state.paths().config).unwrap();
        assert_eq!(store.active().bridge.name, "From backup");
    }

    #[tokio::test]
    async fn listener_closes_after_apply() {
        let fixture = Fixture::new();
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(serve(fixture.state.clone(), listener));

        // reachable while in setup mode
        assert!(tokio::net::TcpStream::connect(addr).await.is_ok());

        let session = fixture.set_up().await;
        fixture
            .call(
                Method::PUT,
                "/bifrost/config/draft",
                Some(&session),
                Some(json!({"revision": 0, "config": minimal_document()})),
            )
            .await;
        let (status, _) = fixture
            .call(
                Method::POST,
                "/bifrost/config/apply",
                Some(&session),
                Some(json!({"revision": 1})),
            )
            .await;
        assert_eq!(status, StatusCode::OK);

        let outcome = server.await.unwrap().unwrap();
        assert_eq!(outcome, SetupOutcome::Configured);
        // the bridge can now bind the same address
        assert!(std::net::TcpListener::bind(addr).is_ok());
    }
}
