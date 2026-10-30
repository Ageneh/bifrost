//! Guard coverage of the whole `/bifrost` nest.
//!
//! The route-enumeration test reads every `.route(..)` under `src/routes`
//! from the source, so a route added in a new file is probed without anyone
//! remembering to list it here.

use std::fs;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request};
use camino::{Utf8Path, Utf8PathBuf};
use hyper::StatusCode;
use svc::manager::ServiceManager;
use tower::ServiceExt;

use crate::config_store::ConfigStore;
use crate::routes;
use crate::server::appstate::AppState;
use crate::server::hue_cors_layer;
use crate::testutil::{MINIMAL_CONFIG, TempDir};

/// What the guard answers a request that carries no credentials.
const UNAUTHENTICATED: StatusCode = StatusCode::FORBIDDEN;

/// Routes that answer without a session, by design.
const PUBLIC_ROUTES: &[(&str, &str)] = &[];

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
    "/bifrost/backend/z2m/{name}",
    "/bifrost/config",
    "/bifrost/config/draft",
    "/bifrost/config/apply",
    "/bifrost/ws",
    "/bifrost/auth/session",
    "/bifrost/auth/login",
    "/bifrost/auth/password",
];

struct Fixture {
    _dir: TempDir,
    router: Router,
}

impl Fixture {
    async fn new() -> Self {
        let dir = TempDir::new();
        let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();

        Self {
            _dir: dir,
            router: routes::router(state, hue_cors_layer()),
        }
    }

    async fn call(&self, method: Method, uri: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn routes_dir() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/routes")
}

/// String literals passed to `call(` in `source`, e.g. every
/// `.route("/x", ...)` path.
fn literal_arguments(source: &str, call: &str) -> Vec<String> {
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
        if module == "mod" || module == "tests" {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        let routes = literal_arguments(&source, ".route(");
        assert!(
            literal_arguments(&source, ".nest(").is_empty(),
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
fn concrete(path: &str) -> String {
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
    let request = Request::builder()
        .method(Method::OPTIONS)
        .uri(uri)
        .header("origin", "http://evil.example")
        .header("access-control-request-method", "PUT")
        .body(Body::empty())
        .unwrap();
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
        "/bifrost/backend/z2m/x",
    ] {
        assert!(!preflight_allows(&fixture, uri).await, "{uri}");
    }
}
