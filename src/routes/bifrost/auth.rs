//! Login, logout, first-run setup and password change under
//! `/bifrost/auth`.
//!
//! Request bodies hold passwords and setup codes: no handler here logs one,
//! and errors carry fixed messages only.

use std::net::{IpAddr, SocketAddr};

use axum::Router;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::header::{COOKIE, RETRY_AFTER, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hyper::StatusCode;

use bifrost_api::admin::ErrorResponse;
use bifrost_api::admin::auth::{
    LoginRequest, LogoutRequest, PasswordChange, SESSION_COOKIE, SessionResponse, SetupRequest,
};

use crate::auth::{AuthError, Expiry, NewSession, check_password_policy, verify_password};
use crate::config::AppConfig;
use crate::config_store::ConfigStoreError;
use crate::routes::extractor::Json;
use crate::server::Protocol;
use crate::server::appstate::AppState;

/// Path the session cookie is scoped to.
const COOKIE_PATH: &str = "/bifrost";

/// Length of a session token in the cookie (32 bytes, hex).
const TOKEN_HEX_LEN: usize = 64;

/// Marks a request that reached `/bifrost` through a trusted Home Assistant
/// Ingress peer. Only the access guard inserts it.
#[derive(Clone, Copy, Debug)]
pub struct TrustedIngress;

/// Who is asking: the peer address, how the connection was made, and the
/// session token the request presented, if any.
#[derive(Clone, Debug)]
pub struct Client {
    pub ip: IpAddr,
    pub tls: bool,
    pub ingress: bool,
    pub session_token: Option<String>,
}

impl Client {
    #[must_use]
    pub fn from_parts(parts: &Parts) -> Option<Self> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip())?;

        Some(Self {
            ip,
            tls: parts.extensions.get::<Protocol>() == Some(&Protocol::Https),
            ingress: parts.extensions.get::<TrustedIngress>().is_some(),
            session_token: session_token(&parts.headers),
        })
    }

    /// The CSRF token of the presented session, when it is live and bound
    /// to this client's address.
    #[must_use]
    pub fn session(&self, state: &AppState, config: &AppConfig) -> Option<String> {
        let token = self.session_token.as_deref()?;
        state
            .auth()
            .session(token, self.ip, Expiry::from_config(&config.bifrost.auth))
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = AuthApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Self::from_parts(parts).ok_or(AuthApiError::UnknownPeer))
    }
}

/// The session token from the `Cookie` header. Anything that is not shaped
/// like a token is ignored.
#[must_use]
pub fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.trim())
        .filter(|value| {
            value.len() == TOKEN_HEX_LEN && value.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .map(str::to_string)
}

fn cookie(value: &str, max_age_seconds: i64, tls: bool) -> HeaderValue {
    let secure = if tls { "; Secure" } else { "" };
    let cookie = format!(
        "{SESSION_COOKIE}={value}; Path={COOKIE_PATH}; HttpOnly; SameSite=Strict; Max-Age={max_age_seconds}{secure}"
    );
    // the token is hex and every other part is fixed text
    HeaderValue::from_str(&cookie).expect("session cookie is a valid header value")
}

#[derive(Debug)]
pub enum AuthApiError {
    Auth(AuthError),
    Store(ConfigStoreError),
    UnknownPeer,
    Internal(String),
}

impl From<AuthError> for AuthApiError {
    fn from(err: AuthError) -> Self {
        Self::Auth(err)
    }
}

impl From<ConfigStoreError> for AuthApiError {
    fn from(err: ConfigStoreError) -> Self {
        Self::Store(err)
    }
}

impl From<tokio::task::JoinError> for AuthApiError {
    fn from(err: tokio::task::JoinError) -> Self {
        Self::Internal(err.to_string())
    }
}

impl IntoResponse for AuthApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Auth(
                AuthError::WrongPassword | AuthError::WrongSetupCode | AuthError::NotAuthenticated,
            ) => StatusCode::UNAUTHORIZED,
            Self::Auth(AuthError::RateLimited(_)) => StatusCode::TOO_MANY_REQUESTS,
            Self::Auth(AuthError::UnacceptablePassword) => StatusCode::BAD_REQUEST,
            Self::Auth(AuthError::AlreadySetUp | AuthError::SetupRequired) => StatusCode::CONFLICT,
            Self::Auth(AuthError::Hashing | AuthError::Io(_) | AuthError::Yaml(_))
            | Self::Store(_)
            | Self::UnknownPeer
            | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let error = match &self {
            Self::Auth(err) => err.to_string(),
            Self::Store(err) => err.to_string(),
            Self::UnknownPeer => "The client address is unknown".to_string(),
            Self::Internal(err) => err.clone(),
        };

        if status.is_server_error() {
            log::error!("Authentication request failed: {error}");
        }

        let retry_after = match &self {
            Self::Auth(AuthError::RateLimited(wait)) => Some(wait.as_secs().max(1)),
            _ => None,
        };

        let mut response = (
            status,
            Json(ErrorResponse {
                error,
                errors: vec![],
            }),
        )
            .into_response();
        if let Some(seconds) = retry_after {
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

type AuthApiResult<T> = Result<T, AuthApiError>;

const fn session_response(
    config: &AppConfig,
    client: &Client,
    csrf: Option<String>,
) -> SessionResponse {
    SessionResponse {
        authenticated: csrf.is_some() || client.ingress,
        setup_required: config.bifrost.auth.password_hash.is_none(),
        ingress: client.ingress,
        csrf,
    }
}

/// The reply to a request that started `session`: its cookie, and the
/// session state with the CSRF token.
fn new_session_reply(config: &AppConfig, client: &Client, session: NewSession) -> Response {
    let max_age = Expiry::from_config(&config.bifrost.auth)
        .absolute
        .num_seconds();
    let body = session_response(config, client, Some(session.csrf));
    (
        [(SET_COOKIE, cookie(&session.token, max_age, client.tls))],
        Json(body),
    )
        .into_response()
}

/// Verify `password` against the configured hash off the executor, counting
/// a failure against the client's backoff.
async fn check_password(
    state: &AppState,
    config: &AppConfig,
    client: &Client,
    password: String,
) -> AuthApiResult<()> {
    let auth = state.auth();
    auth.check_backoff(client.ip)?;

    let hash = config
        .bifrost
        .auth
        .password_hash
        .clone()
        .ok_or(AuthError::SetupRequired)?;

    if tokio::task::spawn_blocking(move || verify_password(&password, &hash)).await? {
        auth.record_success(client.ip);
        Ok(())
    } else {
        auth.record_failure(client.ip);
        log::warn!("Wrong admin password from {}", client.ip);
        Err(AuthError::WrongPassword.into())
    }
}

/// Hash a new password off the executor.
async fn hash_new_password(state: &AppState, password: String) -> AuthApiResult<String> {
    check_password_policy(&password)?;
    let auth = state.auth();
    Ok(tokio::task::spawn_blocking(move || auth.hash_password(&password)).await??)
}

async fn get_session(State(state): State<AppState>, client: Client) -> Json<SessionResponse> {
    let config = state.config();
    let csrf = client.session(&state, &config);
    Json(session_response(&config, &client, csrf))
}

async fn post_setup(
    State(state): State<AppState>,
    client: Client,
    Json(req): Json<SetupRequest>,
) -> AuthApiResult<Response> {
    let auth = state.auth();
    auth.check_backoff(client.ip)?;

    if state.config().bifrost.auth.password_hash.is_some() {
        return Err(AuthError::AlreadySetUp.into());
    }

    // Home Assistant authenticated an Ingress user already; everyone else
    // proves access to the console the setup code was printed on
    if !client.ingress && !auth.check_setup_code(req.setup_code.as_deref().unwrap_or_default()) {
        auth.record_failure(client.ip);
        log::warn!("Wrong setup code from {}", client.ip);
        return Err(AuthError::WrongSetupCode.into());
    }

    let hash = hash_new_password(&state, req.password).await?;
    let set = state.store().update_auth(|config| {
        if config.password_hash.is_some() {
            return false;
        }
        config.password_hash = Some(hash);
        true
    })?;
    if !set {
        return Err(AuthError::AlreadySetUp.into());
    }

    auth.clear_setup_code();
    auth.record_success(client.ip);
    auth.revoke_all()?;
    log::info!("Admin password set from {}", client.ip);

    let config = state.config();
    let session = auth.create_session(client.ip, Expiry::from_config(&config.bifrost.auth))?;
    Ok(new_session_reply(&config, &client, session))
}

async fn post_login(
    State(state): State<AppState>,
    client: Client,
    Json(req): Json<LoginRequest>,
) -> AuthApiResult<Response> {
    let config = state.config();
    check_password(&state, &config, &client, req.password).await?;

    let auth = state.auth();
    // a new token on every login, so a token planted before login is useless
    if let Some(old) = &client.session_token {
        auth.revoke(old)?;
    }
    let session = auth.create_session(client.ip, Expiry::from_config(&config.bifrost.auth))?;
    log::info!("Admin login from {}", client.ip);

    Ok(new_session_reply(&config, &client, session))
}

async fn post_logout(
    State(state): State<AppState>,
    client: Client,
    Json(req): Json<LogoutRequest>,
) -> AuthApiResult<Response> {
    let config = state.config();
    if client.session(&state, &config).is_none() && !client.ingress {
        return Err(AuthError::NotAuthenticated.into());
    }

    let auth = state.auth();
    if req.all {
        auth.revoke_all()?;
        log::info!("All admin sessions ended from {}", client.ip);
    } else if let Some(token) = &client.session_token {
        auth.revoke(token)?;
    }

    Ok((
        [(SET_COOKIE, cookie("", 0, client.tls))],
        Json(session_response(&config, &client, None)),
    )
        .into_response())
}

async fn post_password(
    State(state): State<AppState>,
    client: Client,
    Json(req): Json<PasswordChange>,
) -> AuthApiResult<Response> {
    let config = state.config();
    if client.session(&state, &config).is_none() && !client.ingress {
        return Err(AuthError::NotAuthenticated.into());
    }

    // required under Ingress too: a spoofed Ingress peer must not be able
    // to replace the password
    check_password(&state, &config, &client, req.current_password).await?;
    let hash = hash_new_password(&state, req.new_password).await?;
    state.store().update_auth(|config| {
        config.password_hash = Some(hash);
        true
    })?;

    let auth = state.auth();
    auth.revoke_all()?;
    log::info!("Admin password changed from {}", client.ip);

    let config = state.config();
    if client.ingress {
        return Ok(Json(session_response(&config, &client, None)).into_response());
    }
    let session = auth.create_session(client.ip, Expiry::from_config(&config.bifrost.auth))?;
    Ok(new_session_reply(&config, &client, session))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/session", get(get_session))
        .route("/setup", post(post_setup))
        .route("/login", post(post_login))
        .route("/logout", post(post_logout))
        .route("/password", post(post_password))
}

#[cfg(test)]
mod tests {
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

    use crate::auth::{Auth, verify_password};
    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::auth::{TrustedIngress, router};
    use crate::server::Protocol;
    use crate::server::appstate::AppState;
    use crate::testutil::{
        FAST_HASH, MINIMAL_CONFIG, ManualClock, TempDir, capture_logs, captured_logs,
    };

    const OWNER: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 12, 0, 50));
    const STRANGER: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 12, 0, 66));

    const PASSWORD: &str = "w2-owner-password-7731";
    const NEW_PASSWORD: &str = "w2-new-password-9152";

    struct Fixture {
        dir: TempDir,
        state: AppState,
        router: Router,
        setup_code: String,
        responses: std::sync::Mutex<Vec<String>>,
    }

    struct Reply {
        status: StatusCode,
        body: Value,
        cookie: Option<String>,
        retry_after: Option<String>,
    }

    impl Reply {
        /// The session token the reply set, if it set one.
        fn token(&self) -> Option<String> {
            let cookie = self.cookie.as_deref()?;
            let value = cookie.split(';').next()?.split_once('=')?.1;
            (!value.is_empty()).then(|| value.to_string())
        }
    }

    impl Fixture {
        async fn new() -> Self {
            capture_logs();
            let dir = TempDir::new();
            let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
            let (svm, _future) = ServiceManager::spawn();
            let auth = Arc::new(Auth::load(
                &store.paths().sessions,
                ManualClock::new(),
                FAST_HASH,
            ));
            let state = AppState::from_store(Arc::new(store), svm)
                .await
                .unwrap()
                .with_auth(auth);
            let setup_code = state.auth().issue_setup_code();

            Self {
                dir,
                router: router().with_state(state.clone()),
                state,
                setup_code,
                responses: std::sync::Mutex::new(vec![]),
            }
        }

        async fn send(
            &self,
            method: Method,
            uri: &str,
            from: IpAddr,
            token: Option<&str>,
            body: Option<Value>,
            configure: impl FnOnce(&mut Request<Body>) + Send,
        ) -> Reply {
            let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json");
            if let Some(token) = token {
                request = request.header("cookie", format!("other=1; bifrost_session={token}"));
            }
            let mut request = request.body(body).unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::new(from, 50000)));
            configure(&mut request);

            let response = self.router.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let (parts, body) = response.into_parts();
            let header = |name: &str| {
                parts
                    .headers
                    .get(name)
                    .map(|value| value.to_str().unwrap().to_string())
            };
            let cookie = header("set-cookie");
            let retry_after = header("retry-after");
            let bytes = to_bytes(body, usize::MAX).await.unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            self.responses.lock().unwrap().push(text.clone());

            Reply {
                status,
                body: serde_json::from_str(&text).unwrap_or(Value::String(text)),
                cookie,
                retry_after,
            }
        }

        async fn post(&self, uri: &str, from: IpAddr, token: Option<&str>, body: Value) -> Reply {
            self.send(Method::POST, uri, from, token, Some(body), |_| {})
                .await
        }

        async fn session(&self, from: IpAddr, token: Option<&str>) -> Value {
            let reply = self
                .send(Method::GET, "/session", from, token, None, |_| {})
                .await;
            assert_eq!(reply.status, StatusCode::OK);
            reply.body
        }

        async fn set_up(&self) -> String {
            let reply = self
                .post(
                    "/setup",
                    OWNER,
                    None,
                    json!({"setup_code": self.setup_code, "password": PASSWORD}),
                )
                .await;
            assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
            reply.token().unwrap()
        }

        async fn login(&self, from: IpAddr, password: &str) -> Reply {
            self.post("/login", from, None, json!({"password": password}))
                .await
        }

        /// No secret this fixture handled appears in a response or a log
        /// line.
        fn assert_no_secret_leaked(&self, tokens: &[&str]) {
            let hash = self.state.config().bifrost.auth.password_hash.clone();
            let mut secrets: Vec<&str> = vec![PASSWORD, NEW_PASSWORD, &self.setup_code];
            secrets.extend(tokens);
            secrets.extend(hash.as_deref());
            let logs = captured_logs();
            let responses = self.responses.lock().unwrap().join("\n");
            for secret in secrets {
                assert!(!logs.contains(secret), "secret in logs: {secret}");
                assert!(
                    !responses.contains(secret),
                    "secret in a response: {secret}"
                );
            }
        }
    }

    #[tokio::test]
    async fn fresh_install_requires_setup() {
        let fixture = Fixture::new().await;

        let session = fixture.session(OWNER, None).await;

        assert_eq!(
            session,
            json!({"authenticated": false, "setup_required": true, "ingress": false, "csrf": null})
        );
        let reply = fixture.login(OWNER, PASSWORD).await;
        assert_eq!(reply.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn setup_needs_the_code_and_starts_a_session() {
        let fixture = Fixture::new().await;

        for code in [json!(null), json!(""), json!("AAAA-BBBB-CCCC")] {
            let reply = fixture
                .post(
                    "/setup",
                    OWNER,
                    None,
                    json!({"setup_code": code, "password": PASSWORD}),
                )
                .await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{code}");
        }
        assert!(fixture.state.config().bifrost.auth.password_hash.is_none());

        let reply = fixture
            .post(
                "/setup",
                OWNER,
                None,
                json!({"setup_code": fixture.setup_code.to_lowercase(), "password": PASSWORD}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        let cookie = reply.cookie.clone().unwrap();
        assert!(cookie.contains("; Path=/bifrost; HttpOnly; SameSite=Strict; Max-Age=43200"));
        assert!(!cookie.contains("Secure"));
        let token = reply.token().unwrap();
        assert_eq!(reply.body["authenticated"], true);
        assert_eq!(reply.body["setup_required"], false);

        let hash = fixture
            .state
            .config()
            .bifrost
            .auth
            .password_hash
            .clone()
            .unwrap();
        assert!(verify_password(PASSWORD, &hash));
        let written = std::fs::read_to_string(fixture.dir.path().join("config.yaml")).unwrap();
        assert!(written.contains(&hash));

        let session = fixture.session(OWNER, Some(&token)).await;
        assert_eq!(session["authenticated"], true);
        assert_eq!(session["csrf"], reply.body["csrf"]);

        // the code is single-use, and setup cannot run twice
        let again = fixture
            .post(
                "/setup",
                OWNER,
                None,
                json!({"setup_code": fixture.setup_code, "password": NEW_PASSWORD}),
            )
            .await;
        assert_eq!(again.status, StatusCode::CONFLICT);
        assert!(!fixture.state.auth().check_setup_code(&fixture.setup_code));

        fixture.assert_no_secret_leaked(&[&token]);
    }

    #[tokio::test]
    async fn setup_refuses_a_weak_password() {
        let fixture = Fixture::new().await;

        let reply = fixture
            .post(
                "/setup",
                OWNER,
                None,
                json!({"setup_code": fixture.setup_code, "password": "short"}),
            )
            .await;

        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert!(fixture.state.config().bifrost.auth.password_hash.is_none());
        assert!(fixture.state.auth().check_setup_code(&fixture.setup_code));
    }

    #[tokio::test]
    async fn trusted_ingress_sets_up_without_the_code() {
        let fixture = Fixture::new().await;

        let reply = fixture
            .send(
                Method::POST,
                "/setup",
                OWNER,
                None,
                Some(json!({"password": PASSWORD})),
                |request| {
                    request.extensions_mut().insert(TrustedIngress);
                },
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["ingress"], true);
        assert!(fixture.state.config().bifrost.auth.password_hash.is_some());
    }

    #[tokio::test]
    async fn secure_cookie_over_tls() {
        let fixture = Fixture::new().await;

        let reply = fixture
            .send(
                Method::POST,
                "/setup",
                OWNER,
                None,
                Some(json!({"setup_code": fixture.setup_code, "password": PASSWORD})),
                |request| {
                    request.extensions_mut().insert(Protocol::Https);
                },
            )
            .await;

        assert!(reply.cookie.unwrap().ends_with("; Secure"));
    }

    #[tokio::test]
    async fn login_rotates_the_session() {
        let fixture = Fixture::new().await;
        let first = fixture.set_up().await;

        let reply = fixture
            .post("/login", OWNER, Some(&first), json!({"password": PASSWORD}))
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        let second = reply.token().unwrap();

        assert_ne!(first, second);
        assert_eq!(
            fixture.session(OWNER, Some(&first)).await["authenticated"],
            false
        );
        assert_eq!(
            fixture.session(OWNER, Some(&second)).await["authenticated"],
            true
        );
        fixture.assert_no_secret_leaked(&[&first, &second]);
    }

    #[tokio::test]
    async fn session_is_refused_from_another_address() {
        let fixture = Fixture::new().await;
        let token = fixture.set_up().await;

        let session = fixture.session(STRANGER, Some(&token)).await;

        assert_eq!(session["authenticated"], false);
        assert_eq!(session["csrf"], Value::Null);
        assert_eq!(
            fixture.session(OWNER, Some(&token)).await["authenticated"],
            true
        );
    }

    #[tokio::test]
    async fn failed_logins_back_off_per_address() {
        let fixture = Fixture::new().await;
        fixture.set_up().await;

        for _ in 0..4 {
            let reply = fixture.login(STRANGER, "wrong-password-1").await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
            assert!(reply.cookie.is_none());
        }

        let reply = fixture.login(STRANGER, PASSWORD).await;
        assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(reply.retry_after.as_deref(), Some("1"));

        // never a global lockout: the owner's address is unaffected
        let reply = fixture.login(OWNER, PASSWORD).await;
        assert_eq!(reply.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn sessions_survive_a_restart() {
        let fixture = Fixture::new().await;
        let token = fixture.set_up().await;

        // a new process: the store and the session file are read again
        let store = ConfigStore::load(&fixture.dir.path().join("config.yaml")).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();
        let restarted = router().with_state(state);

        let mut request = Request::builder()
            .uri("/session")
            .header("cookie", format!("bifrost_session={token}"))
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(OWNER, 1)));
        let response = restarted.oneshot(request).await.unwrap();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();

        assert_eq!(body["authenticated"], true);
    }

    #[tokio::test]
    async fn deleting_the_password_resets_access() {
        let fixture = Fixture::new().await;
        let token = fixture.set_up().await;

        // the documented reset: stop, delete the password_hash line, restart
        let path = fixture.dir.path().join("config.yaml");
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .filter(|line| !line.contains("password_hash"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, text).unwrap();
        let store = ConfigStore::load(&path).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();

        assert!(state.config().bifrost.auth.password_hash.is_none());
        let expiry = crate::auth::Expiry::from_config(&state.config().bifrost.auth);
        assert!(state.auth().session(&token, OWNER, expiry).is_none());
    }

    #[tokio::test]
    async fn password_change_needs_the_current_password_and_logs_everyone_out() {
        let fixture = Fixture::new().await;
        let owner = fixture.set_up().await;
        let other = fixture.login(STRANGER, PASSWORD).await.token().unwrap();

        let reply = fixture
            .post(
                "/password",
                OWNER,
                None,
                json!({"current_password": PASSWORD, "new_password": NEW_PASSWORD}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

        let reply = fixture
            .post(
                "/password",
                OWNER,
                Some(&owner),
                json!({"current_password": "not-the-password", "new_password": NEW_PASSWORD}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

        let reply = fixture
            .post(
                "/password",
                OWNER,
                Some(&owner),
                json!({"current_password": PASSWORD, "new_password": NEW_PASSWORD}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        let rotated = reply.token().unwrap();

        assert_eq!(
            fixture.session(OWNER, Some(&owner)).await["authenticated"],
            false
        );
        assert_eq!(
            fixture.session(STRANGER, Some(&other)).await["authenticated"],
            false
        );
        assert_eq!(
            fixture.session(OWNER, Some(&rotated)).await["authenticated"],
            true
        );
        assert_eq!(
            fixture.login(OWNER, PASSWORD).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            fixture.login(STRANGER, NEW_PASSWORD).await.status,
            StatusCode::OK
        );

        fixture.assert_no_secret_leaked(&[&owner, &other, &rotated]);
    }

    #[tokio::test]
    async fn ingress_password_change_still_needs_the_current_password() {
        let fixture = Fixture::new().await;
        fixture.set_up().await;

        let reply = fixture
            .send(
                Method::POST,
                "/password",
                OWNER,
                None,
                Some(json!({"current_password": "guess-guess", "new_password": NEW_PASSWORD})),
                |request| {
                    request.extensions_mut().insert(TrustedIngress);
                },
            )
            .await;

        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_ends_the_session_and_clears_the_cookie() {
        let fixture = Fixture::new().await;
        let token = fixture.set_up().await;
        let other = fixture.login(OWNER, PASSWORD).await.token().unwrap();

        let reply = fixture
            .post("/logout", OWNER, Some(&token), json!({}))
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert!(
            reply
                .cookie
                .unwrap()
                .contains("bifrost_session=; Path=/bifrost")
        );
        assert_eq!(
            fixture.session(OWNER, Some(&token)).await["authenticated"],
            false
        );
        assert_eq!(
            fixture.session(OWNER, Some(&other)).await["authenticated"],
            true
        );

        let reply = fixture
            .post("/logout", OWNER, Some(&other), json!({"all": true}))
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(
            fixture
                .state
                .auth()
                .session_count(crate::auth::Expiry::from_config(
                    &fixture.state.config().bifrost.auth
                )),
            0
        );

        let reply = fixture
            .post("/logout", OWNER, Some(&other), json!({}))
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_cookies_are_ignored() {
        let fixture = Fixture::new().await;
        fixture.set_up().await;

        for token in ["", "x", "../../etc", &"g".repeat(64)] {
            let session = fixture.session(OWNER, Some(token)).await;
            assert_eq!(session["authenticated"], false, "{token}");
        }
    }
}
