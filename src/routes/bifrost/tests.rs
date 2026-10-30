//! Guard coverage of the whole `/bifrost` nest.
//!
//! The route-enumeration test reads every `.route(..)` under `src/routes`
//! from the source, so a route added in a new file is probed without anyone
//! remembering to list it here.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Method, Request};
use camino::{Utf8Path, Utf8PathBuf};
use hyper::StatusCode;
use serde_json::{Value, json};
use svc::manager::ServiceManager;
use tower::ServiceExt;

use crate::auth::{Auth, Expiry, NewSession};
use crate::config_store::ConfigStore;
use crate::routes;
use crate::server::appstate::AppState;
use crate::server::hue_cors_layer;
use crate::testutil::{FAST_HASH, MINIMAL_CONFIG, ManualClock, TempDir};

/// What the guard answers a request that carries no session.
const UNAUTHENTICATED: StatusCode = StatusCode::UNAUTHORIZED;

/// Routes that answer without a session, by design: how one is obtained.
const PUBLIC_ROUTES: &[(&str, &str)] = &[
    ("GET", "/bifrost/auth/session"),
    ("POST", "/bifrost/auth/login"),
    ("POST", "/bifrost/auth/setup"),
];

const METHODS: &[Method] = &[
    Method::GET,
    Method::POST,
    Method::PUT,
    Method::PATCH,
    Method::DELETE,
];

/// Routes that exist today; the scan must find them, or it is broken.
const KNOWN_ROUTES: &[&str] = &[
    "/bifrost/service",
    "/bifrost/service/{id}",
    "/bifrost/service/{id}/restart",
    "/bifrost/bridge/certificate",
    "/bifrost/bridge/certificate/regenerate",
    "/bifrost/z2m",
    "/bifrost/z2m/{name}",
    "/bifrost/config",
    "/bifrost/config/draft",
    "/bifrost/config/apply",
    "/bifrost/logs",
    "/bifrost/state/summary",
    "/bifrost/state/export",
    "/bifrost/state/reset",
    "/bifrost/ws",
    "/bifrost/auth/session",
    "/bifrost/auth/login",
    "/bifrost/auth/password",
    "/bifrost/entertainment/capabilities",
];

/// `bridge.ipaddress` of [`MINIMAL_CONFIG`].
const BRIDGE_HOST: &str = "10.12.0.20";
const SAME_ORIGIN: &str = "http://10.12.0.20";

const OWNER: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 12, 0, 50));
const SUPERVISOR: IpAddr = IpAddr::V4(Ipv4Addr::new(172, 30, 32, 2));
/// Another add-on on the Supervisor network: not the Ingress proxy.
const CO_LOCATED_ADDON: IpAddr = IpAddr::V4(Ipv4Addr::new(172, 30, 32, 5));

struct Fixture {
    _dir: TempDir,
    state: AppState,
    router: Router,
}

impl Fixture {
    async fn new() -> Self {
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

        Self {
            _dir: dir,
            router: routes::router(state.clone(), hue_cors_layer()),
            state,
        }
    }

    fn login(&self) -> NewSession {
        let expiry = Expiry::from_config(&self.state.config().bifrost.auth);
        self.state.auth().create_session(OWNER, expiry).unwrap()
    }

    /// Send a request from `from`. `Host` is the bridge address unless
    /// `headers` names one; a `host` of `""` sends none.
    async fn send(
        &self,
        method: Method,
        uri: &str,
        from: IpAddr,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if !headers.iter().any(|(name, _)| *name == "host") {
            request = request.header("host", BRIDGE_HOST);
        }
        for (name, value) in headers {
            if !value.is_empty() {
                request = request.header(*name, *value);
            }
        }
        let body = body.unwrap_or_else(|| json!({}));
        let mut request = request.body(Body::from(body.to_string())).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(from, 50000)));

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn call(&self, method: Method, uri: &str) -> (StatusCode, String) {
        self.send(method, uri, OWNER, &[], None).await
    }

    /// A draft write that changes nothing but the revision; succeeds when
    /// the guard lets it through.
    async fn patch_draft(&self, from: IpAddr, headers: &[(&str, &str)]) -> (StatusCode, String) {
        let revision = self.state.store().revision();
        self.send(
            Method::PATCH,
            "/bifrost/config/draft",
            from,
            headers,
            Some(json!({"revision": revision, "patch": {}})),
        )
        .await
    }
}

pub(crate) fn routes_dir() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/routes")
}

/// String literals passed to `call(` in `source`, e.g. every
/// `.route("/x", ...)` path.
pub(crate) fn literal_arguments(source: &str, call: &str) -> Vec<String> {
    let mut found = vec![];
    let mut rest = source;
    while let Some(offset) = rest.find(call) {
        rest = &rest[offset + call.len()..];
        let trimmed = rest.trim_start();
        if let Some(literal) = trimmed.strip_prefix('"') {
            if let Some(end) = literal.find('"') {
                found.push(literal[..end].to_string());
            }
        }
    }
    found
}

/// `.nest("/prefix", module::router())` pairs in `source`.
fn nested_modules(source: &str) -> Vec<(String, String)> {
    let mut found = vec![];
    let mut rest = source;
    while let Some(offset) = rest.find(".nest(") {
        rest = &rest[offset + ".nest(".len()..];
        let trimmed = rest.trim_start();
        let Some(literal) = trimmed.strip_prefix('"') else {
            continue;
        };
        let Some(end) = literal.find('"') else {
            continue;
        };
        let prefix = literal[..end].to_string();
        let after = literal[end + 1..].trim_start_matches([',', ' ', '\n']);
        if let Some(module) = after.split("::router").next() {
            found.push((prefix, module.trim().to_string()));
        }
    }
    found
}

fn join(prefix: &str, path: &str) -> String {
    if path == "/" {
        prefix.to_string()
    } else {
        format!("{prefix}{path}")
    }
}

/// Every route path mounted at or below `/bifrost`, read from the sources
/// under `src/routes`.
fn scan_bifrost_routes() -> Vec<String> {
    let bifrost_dir = routes_dir().join("bifrost");
    let mod_source = fs::read_to_string(bifrost_dir.join("mod.rs")).unwrap();

    let mut paths: Vec<String> = literal_arguments(&mod_source, ".route(")
        .iter()
        .map(|path| join("/bifrost", path))
        .collect();

    let prefixes = nested_modules(&mod_source);

    for entry in fs::read_dir(&bifrost_dir).unwrap() {
        let path = Utf8PathBuf::from_path_buf(entry.unwrap().path()).unwrap();
        let module = path.file_stem().unwrap().to_string();
        // setup mode is a router of its own; its tests enumerate it
        if module == "mod" || module == "tests" || module == "setup" {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        // the routes, not a test module's request strings
        let source = source
            .find("#[cfg(test)]")
            .map_or(source.as_str(), |end| &source[..end]);
        let routes = literal_arguments(source, ".route(");
        assert!(
            literal_arguments(source, ".nest(").is_empty(),
            "{path}: nested routers below /bifrost/{module} are not understood by this scan; extend it"
        );
        if routes.is_empty() {
            continue;
        }
        let (prefix, _) = prefixes
            .iter()
            .find(|(_, name)| *name == module)
            .unwrap_or_else(|| panic!("{path} declares routes, but /bifrost does not nest it"));
        paths.extend(
            routes
                .iter()
                .map(|route| join(&join("/bifrost", prefix), route)),
        );
    }

    // a /bifrost path mounted from anywhere else in src/routes
    scan_outside(&routes_dir(), &bifrost_dir, &mut paths);

    paths.sort();
    paths.dedup();
    paths
}

fn scan_outside(dir: &Utf8Path, skip: &Utf8Path, paths: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = Utf8PathBuf::from_path_buf(entry.unwrap().path()).unwrap();
        if path == skip {
            continue;
        }
        if path.is_dir() {
            scan_outside(&path, skip, paths);
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for call in [".route(", ".route_service(", ".nest_service("] {
            paths.extend(
                literal_arguments(&source, call)
                    .into_iter()
                    .filter(|route| route.starts_with("/bifrost")),
            );
        }
    }
}

/// A concrete uri for a route path with `{param}` captures.
pub(crate) fn concrete(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with('{') {
                "00000000-0000-0000-0000-000000000000"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn scan_finds_the_known_routes() {
    let paths = scan_bifrost_routes();
    for known in KNOWN_ROUTES {
        assert!(
            paths.iter().any(|path| path == known),
            "{known} not in {paths:?}"
        );
    }
    for (_, public) in PUBLIC_ROUTES {
        assert!(
            paths.iter().any(|path| path == public),
            "{public} not in {paths:?}"
        );
    }
}

#[tokio::test]
async fn every_bifrost_route_is_guarded() {
    let fixture = Fixture::new().await;

    let mut probed = 0;
    for path in scan_bifrost_routes() {
        for method in METHODS {
            if PUBLIC_ROUTES.contains(&(method.as_str(), path.as_str())) {
                continue;
            }
            let (status, body) = fixture.call(method.clone(), &concrete(&path)).await;
            assert_eq!(status, UNAUTHENTICATED, "{method} {path}: {body}");
            probed += 1;
        }
    }
    assert!(probed >= KNOWN_ROUTES.len() * METHODS.len());
}

#[tokio::test]
async fn unknown_bifrost_paths_are_guarded_and_never_html() {
    let fixture = Fixture::new().await;

    for uri in ["/bifrost/nope", "/bifrost/config/nope", "/bifrost"] {
        let (status, body) = fixture.call(Method::GET, uri).await;
        assert_eq!(status, UNAUTHENTICATED, "{uri}: {body}");
        assert!(!body.contains("<html"), "{uri}: {body}");
    }
}

async fn preflight_allows(fixture: &Fixture, uri: &str) -> bool {
    let mut request = Request::builder()
        .method(Method::OPTIONS)
        .uri(uri)
        .header("host", BRIDGE_HOST)
        .header("origin", "http://evil.example")
        .header("access-control-request-method", "PUT")
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(OWNER, 50000)));
    let response = fixture.router.clone().oneshot(request).await.unwrap();
    response
        .headers()
        .contains_key("access-control-allow-origin")
}

#[tokio::test]
async fn cors_covers_hue_routes_only() {
    let fixture = Fixture::new().await;

    assert!(preflight_allows(&fixture, "/api/config").await);
    assert!(preflight_allows(&fixture, "/clip/v2/resource/light").await);

    for uri in [
        "/bifrost/config",
        "/bifrost/service/00000000-0000-0000-0000-000000000000",
        "/bifrost/z2m/x",
    ] {
        assert!(!preflight_allows(&fixture, uri).await, "{uri}");
    }
}

#[tokio::test]
async fn public_routes_answer_without_a_session() {
    let fixture = Fixture::new().await;

    let (status, body) = fixture.call(Method::GET, "/bifrost/auth/session").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // a public mutation still needs a same-origin request
    let password = json!({"password": "whatever-it-is"});
    let (status, _) = fixture
        .send(
            Method::POST,
            "/bifrost/auth/login",
            OWNER,
            &[],
            Some(password.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture
        .send(
            Method::POST,
            "/bifrost/auth/login",
            OWNER,
            &[("origin", SAME_ORIGIN)],
            Some(password),
        )
        .await;
    // through the guard: no password is set yet
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn mutation_needs_session_then_csrf_then_origin() {
    let fixture = Fixture::new().await;
    let session = fixture.login();
    let cookie = format!("bifrost_session={}", session.token);

    // no session
    let (status, _) = fixture
        .patch_draft(
            OWNER,
            &[("origin", SAME_ORIGIN), ("x-bifrost-csrf", &session.csrf)],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // session, no CSRF token
    let (status, body) = fixture
        .patch_draft(OWNER, &[("cookie", &cookie), ("origin", SAME_ORIGIN)])
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("X-Bifrost-Csrf"), "{body}");

    // session, wrong CSRF token (another session's)
    let other = fixture.login();
    let (status, _) = fixture
        .patch_draft(
            OWNER,
            &[
                ("cookie", &cookie),
                ("origin", SAME_ORIGIN),
                ("x-bifrost-csrf", &other.csrf),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // session and CSRF token, but a foreign, missing or other-port origin
    for origin in ["http://evil.example", "", "http://10.12.0.20:8123", "null"] {
        let (status, _) = fixture
            .patch_draft(
                OWNER,
                &[
                    ("cookie", &cookie),
                    ("origin", origin),
                    ("x-bifrost-csrf", &session.csrf),
                ],
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin:?}");
    }

    // all three
    let (status, body) = fixture
        .patch_draft(
            OWNER,
            &[
                ("cookie", &cookie),
                ("origin", SAME_ORIGIN),
                ("x-bifrost-csrf", &session.csrf),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // a Referer stands in for a missing Origin
    let (status, body) = fixture
        .patch_draft(
            OWNER,
            &[
                ("cookie", &cookie),
                ("referer", "http://10.12.0.20/#/settings"),
                ("x-bifrost-csrf", &session.csrf),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn reads_need_a_session_but_no_csrf() {
    let fixture = Fixture::new().await;
    let session = fixture.login();
    let cookie = format!("bifrost_session={}", session.token);

    let (status, _) = fixture.call(Method::GET, "/bifrost/config").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = fixture
        .send(
            Method::GET,
            "/bifrost/config",
            OWNER,
            &[("cookie", &cookie)],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn session_cookie_is_useless_from_another_address() {
    let fixture = Fixture::new().await;
    let session = fixture.login();
    let cookie = format!("bifrost_session={}", session.token);

    let (status, _) = fixture
        .send(
            Method::GET,
            "/bifrost/config",
            IpAddr::V4(Ipv4Addr::new(10, 12, 0, 99)),
            &[("cookie", &cookie)],
            None,
        )
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_host_names_are_refused() {
    let fixture = Fixture::new().await;
    let session = fixture.login();
    let cookie = format!("bifrost_session={}", session.token);

    // a rebound attacker domain, even with a valid session
    for host in ["evil.example", "evil.example:80", ""] {
        let (status, _) = fixture
            .send(
                Method::GET,
                "/bifrost/config",
                OWNER,
                &[("cookie", &cookie), ("host", host)],
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{host:?}");
    }

    for host in [
        "10.12.0.20:80",
        "bifrost-334455.local",
        "BIFROST-334455.LOCAL:8080",
    ] {
        let (status, body) = fixture
            .send(
                Method::GET,
                "/bifrost/config",
                OWNER,
                &[("cookie", &cookie), ("host", host)],
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{host}: {body}");
    }

    fixture
        .state
        .store()
        .update_auth(|auth| {
            auth.allowed_hosts.push("bifrost.home.lan".into());
            true
        })
        .unwrap();
    let (status, _) = fixture
        .send(
            Method::GET,
            "/bifrost/config",
            OWNER,
            &[("cookie", &cookie), ("host", "bifrost.home.lan")],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn only_the_supervisor_skips_login() {
    let fixture = Fixture::new().await;
    let ingress = [
        ("x-ingress-path", "/api/hassio_ingress/abc"),
        ("host", "homeassistant.local:8123"),
    ];

    let (status, body) = fixture
        .send(Method::GET, "/bifrost/config", SUPERVISOR, &ingress, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = fixture
        .send(
            Method::GET,
            "/bifrost/auth/session",
            SUPERVISOR,
            &ingress,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let session: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(session["ingress"], true);
    assert_eq!(session["authenticated"], true);

    let (status, body) = fixture.patch_draft(SUPERVISOR, &ingress).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // the header from anyone else is refused outright, forwarding headers
    // or not
    for peer in [CO_LOCATED_ADDON, OWNER] {
        let headers = [
            ingress[0],
            ingress[1],
            ("x-forwarded-for", "172.30.32.2"),
            ("x-real-ip", "172.30.32.2"),
        ];
        let (status, _) = fixture
            .send(Method::GET, "/bifrost/config", peer, &headers, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{peer}");
    }

    // the Supervisor without the Ingress header is an ordinary client
    let (status, _) = fixture
        .send(Method::GET, "/bifrost/config", SUPERVISOR, &[], None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    fixture
        .state
        .store()
        .update_auth(|auth| {
            auth.trust_ingress = false;
            true
        })
        .unwrap();
    let (status, _) = fixture
        .send(Method::GET, "/bifrost/config", SUPERVISOR, &ingress, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn websocket_upgrade_needs_session_and_origin() {
    let fixture = Fixture::new().await;
    let session = fixture.login();
    let cookie = format!("bifrost_session={}", session.token);
    let upgrade = [
        ("connection", "upgrade"),
        ("upgrade", "websocket"),
        ("sec-websocket-version", "13"),
        ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
    ];

    let (status, _) = fixture
        .send(
            Method::GET,
            "/bifrost/ws",
            OWNER,
            &[upgrade[0], upgrade[1], ("origin", SAME_ORIGIN)],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = fixture
        .send(
            Method::GET,
            "/bifrost/ws",
            OWNER,
            &[
                upgrade[0],
                upgrade[1],
                ("cookie", &cookie),
                ("origin", "http://evil.example"),
            ],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = fixture
        .send(
            Method::GET,
            "/bifrost/ws",
            OWNER,
            &[upgrade[0], upgrade[1], ("cookie", &cookie)],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// Over a real socket and the full listener stack: the guard must let the
/// upgrade through untouched, and the first frame must hold no secret.
#[tokio::test]
async fn websocket_works_through_the_guard_over_a_socket() {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    use crate::server::{Protocol, build_service};

    const Z2M_TOKEN: &str = "w2-z2m-token-5521";

    let dir = TempDir::new();
    let yaml = MINIMAL_CONFIG
        .replace("ipaddress: 10.12.0.20", "ipaddress: 127.0.0.1")
        .replace(
            "url: ws://10.0.0.100:8080",
            &format!("url: ws://10.0.0.100:8080/api?token={Z2M_TOKEN}"),
        );
    let store = ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap();
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
    let hash = state.auth().hash_password("w2-socket-password").unwrap();
    state
        .store()
        .update_auth(|auth| {
            auth.password_hash = Some(hash.clone());
            true
        })
        .unwrap();
    let localhost = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let session = state
        .auth()
        .create_session(localhost, Expiry::from_config(&state.config().bifrost.auth))
        .unwrap();

    // the production listener, as HttpServer runs it
    let listener = std::net::TcpListener::bind((localhost, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(axum_server::from_tcp(listener).serve(build_service(Protocol::Http, state)));

    let url = format!("ws://127.0.0.1:{port}/bifrost/ws");
    let origin = format!("http://127.0.0.1:{port}");

    // without the cookie the upgrade is refused
    let mut request = url.as_str().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    assert!(tokio_tungstenite::connect_async(request).await.is_err());

    let mut request = url.as_str().into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert("origin", origin.parse().unwrap());
    headers.insert(
        "cookie",
        format!("bifrost_session={}", session.token)
            .parse()
            .unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    let frame = socket.next().await.unwrap().unwrap().into_text().unwrap();
    assert!(frame.contains("AppConfig"), "{frame}");
    for secret in [Z2M_TOKEN, hash.as_str(), &session.token, &session.csrf] {
        assert!(!frame.contains(secret), "secret in the first frame");
    }
}
