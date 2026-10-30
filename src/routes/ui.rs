//! The web UI: the single-page application shell and its static assets.
//!
//! Served as the router fallback, so every route the Hue and admin APIs
//! mount keeps winning. The shell is `index.html` with a `<base href>` and
//! `window.__BIFROST_BASE__` injected from `X-Ingress-Path`, which makes the
//! same build work at `/` on the LAN and under a Home Assistant ingress prefix.

use std::borrow::Cow;
use std::sync::OnceLock;

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use camino::{Utf8Path, Utf8PathBuf};

/// Header Home Assistant's ingress proxy sets to the prefix it serves us under.
pub const INGRESS_PATH_HEADER: &str = "x-ingress-path";

const INDEX: &str = "index.html";

/// Directory the build tool fingerprints its output into; safe to cache forever.
const IMMUTABLE_ASSET_DIR: &str = "_nuxt/";

/// Longest `X-Ingress-Path` accepted; Home Assistant's is about 60 bytes.
const MAX_INGRESS_PATH_LEN: usize = 256;

/// Path prefixes owned by the Hue and admin APIs. The fallback answers these
/// with an empty 404, never with the shell, so a Hue client or a script that
/// mistypes a URL never receives an HTML 200.
const API_PREFIXES: &[&str] = &[
    "/api",
    "/auth",
    "/bifrost",
    "/clip",
    "/description.xml",
    "/eventstream",
    "/licenses",
    "/updater",
];

#[cfg(feature = "ui")]
#[derive(rust_embed::RustEmbed)]
#[folder = "frontend/.output/public"]
#[allow_missing = true]
struct EmbeddedAssets;

static DIRECTORY: OnceLock<Utf8PathBuf> = OnceLock::new();

/// Serve the UI from `dir` instead of the assets embedded in the binary.
///
/// One directory per process, chosen at startup; later calls are ignored.
pub fn serve_from_directory(dir: Utf8PathBuf) {
    if !dir.join(INDEX).is_file() {
        log::warn!("UI directory {dir} has no {INDEX}; the web UI will answer 404");
    }
    if DIRECTORY.set(dir).is_err() {
        log::warn!("UI directory already chosen; ignoring the new one");
    }
}

/// Where the UI files come from.
#[derive(Clone, Copy, Debug)]
pub enum UiSource<'a> {
    /// Built into the binary by the `ui` feature.
    Embedded,
    /// Read from disk on every request, for development and tests.
    Directory(&'a Utf8Path),
}

impl UiSource<'_> {
    fn current() -> UiSource<'static> {
        DIRECTORY
            .get()
            .map_or(UiSource::Embedded, |dir| UiSource::Directory(dir))
    }

    fn get(self, path: &str) -> Option<Cow<'static, [u8]>> {
        // debug builds of rust-embed read from disk too, so both sources
        // need the check
        if !is_plain_relative(path) {
            return None;
        }
        match self {
            #[cfg(feature = "ui")]
            Self::Embedded => {
                <EmbeddedAssets as rust_embed::RustEmbed>::get(path).map(|file| file.data)
            }
            #[cfg(not(feature = "ui"))]
            Self::Embedded => None,
            Self::Directory(dir) => std::fs::read(dir.join(path)).ok().map(Cow::Owned),
        }
    }
}

/// A relative path with no empty, `.` or `..` segments, so it cannot leave
/// the UI directory.
fn is_plain_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."))
}

fn is_api_path(path: &str) -> bool {
    API_PREFIXES.iter().any(|prefix| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// A path whose last segment has an extension names a file, so a miss is a
/// 404 rather than the shell; otherwise a broken asset reference would load
/// as HTML and hide the failure.
fn names_a_file(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
}

/// The prefix the page is served under, from `X-Ingress-Path`, always ending
/// in `/`. A header that is not a plain absolute path is ignored, so nothing
/// a client sends can break out of the attribute or the script it lands in.
fn base_path(headers: &HeaderMap) -> String {
    let prefix = headers
        .get(INGRESS_PATH_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| is_safe_prefix(value))
        .map_or("", |value| value.trim_end_matches('/'));

    format!("{prefix}/")
}

fn is_safe_prefix(value: &str) -> bool {
    value.len() <= MAX_INGRESS_PATH_LEN
        && value.starts_with('/')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~'))
        && value
            .trim_end_matches('/')
            .split('/')
            .skip(1)
            .all(|segment| !matches!(segment, "" | "." | ".."))
}

/// Insert the base element and the base variable as the first children of
/// `<head>`, ahead of anything that resolves a relative URL.
fn inject_base(index: &str, base: &str) -> String {
    // `base` passed `is_safe_prefix`, so it holds no quote, `<` or `&`
    let base_json = serde_json::Value::from(base);
    let tags =
        format!(r#"<base href="{base}"><script>window.__BIFROST_BASE__={base_json};</script>"#);

    // lowercasing ASCII keeps byte offsets, so positions carry over to `index`
    let lower = index.to_ascii_lowercase();
    let head_end = lower
        .match_indices("<head")
        .map(|(start, _)| start)
        .find(|start| {
            lower
                .as_bytes()
                .get(start + "<head".len())
                .is_some_and(|b| *b == b'>' || b.is_ascii_whitespace())
        })
        .and_then(|start| index[start..].find('>').map(|end| start + end + 1));

    // without a <head> the tags open the document, which parsers accept
    let at = head_end.unwrap_or(0);
    format!("{}{tags}{}", &index[..at], &index[at..])
}

fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

fn shell(index: &[u8], headers: &HeaderMap) -> Response {
    let index = String::from_utf8_lossy(index);
    let body = inject_base(&index, &base_path(headers));

    (
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            // the body depends on X-Ingress-Path, so no cache may reuse it
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        ],
        body,
    )
        .into_response()
}

fn asset(path: &str, bytes: Cow<'static, [u8]>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with(IMMUTABLE_ASSET_DIR) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };

    (
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_str(mime.as_ref())
                    .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
            ),
            (CACHE_CONTROL, HeaderValue::from_static(cache)),
            (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        ],
        bytes,
    )
        .into_response()
}

fn ui_missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        "The Bifrost web UI is not included in this build.\n",
    )
        .into_response()
}

/// Answer a request no other route matched.
#[must_use]
pub fn serve(source: UiSource, method: &Method, path: &str, headers: &HeaderMap) -> Response {
    if !matches!(*method, Method::GET | Method::HEAD) || is_api_path(path) {
        return not_found();
    }

    let relative = path.trim_start_matches('/');
    if !relative.is_empty() && relative != INDEX {
        if let Some(bytes) = source.get(relative) {
            return asset(relative, bytes);
        }
        if names_a_file(relative) {
            return not_found();
        }
    }

    source
        .get(INDEX)
        .map_or_else(ui_missing, |index| shell(&index, headers))
}

pub async fn fallback(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    serve(UiSource::current(), &method, uri.path(), &headers)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::extract::ConnectInfo;
    use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
    use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
    use axum::response::Response;
    use camino::Utf8PathBuf;
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use super::{INGRESS_PATH_HEADER, UiSource, base_path, inject_base, serve};
    use crate::config_store::ConfigStore;
    use crate::routes;
    use crate::server::appstate::AppState;
    use crate::server::hue_cors_layer;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    const INDEX: &str = r#"<!DOCTYPE html><html><head><meta charset="utf-8"><script type="module" src="./_nuxt/entry.js"></script></head><body><div id="__nuxt"></div></body></html>"#;
    const INGRESS: &str = "/api/hassio_ingress/Zq3x_Ab-9";

    struct Ui {
        root: TempDir,
    }

    impl Ui {
        /// A built UI: `public/` holds the shell and one fingerprinted asset;
        /// `secret.txt` sits beside it, outside what may be served.
        fn new() -> Self {
            let root = TempDir::new();
            fs::create_dir_all(root.path().join("public/_nuxt")).unwrap();
            let _ = root.write("public/index.html", INDEX);
            let _ = root.write("public/_nuxt/entry.js", "console.log(1)");
            let _ = root.write("public/robots.txt", "User-agent: *");
            let _ = root.write("secret.txt", "do not serve");
            Self { root }
        }

        async fn get(&self, path: &str, ingress: Option<&str>) -> (Response, String) {
            self.request(&Method::GET, path, ingress).await
        }

        async fn request(
            &self,
            method: &Method,
            path: &str,
            ingress: Option<&str>,
        ) -> (Response, String) {
            let mut headers = HeaderMap::new();
            if let Some(value) = ingress {
                headers.insert(INGRESS_PATH_HEADER, HeaderValue::from_str(value).unwrap());
            }
            let dir = self.root.path().join("public");
            split(serve(UiSource::Directory(&dir), method, path, &headers)).await
        }
    }

    async fn split(response: Response) -> (Response, String) {
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        (Response::from_parts(parts, Body::empty()), text)
    }

    fn header<'a>(response: &'a Response, name: &axum::http::HeaderName) -> &'a str {
        response
            .headers()
            .get(name)
            .map_or("", |value| value.to_str().unwrap())
    }

    fn ingress(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(INGRESS_PATH_HEADER, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn base_path_defaults_to_root() {
        assert_eq!(base_path(&HeaderMap::new()), "/");
    }

    #[test]
    fn base_path_takes_the_ingress_prefix() {
        assert_eq!(base_path(&ingress(INGRESS)), format!("{INGRESS}/"));
        assert_eq!(
            base_path(&ingress(&format!("{INGRESS}/"))),
            format!("{INGRESS}/")
        );
        assert_eq!(base_path(&ingress("/")), "/");
    }

    #[test]
    fn base_path_ignores_unsafe_prefixes() {
        for value in [
            "relative/path",
            "/a//b",
            "/a/../b",
            "/a/./b",
            "/x\"><script>alert(1)</script>",
            "/x'onload='alert(1)",
            "/x&amp;",
            "/x y",
            "//evil.example/",
            "https://evil.example/",
        ] {
            assert_eq!(base_path(&ingress(value)), "/", "accepted {value:?}");
        }
        assert_eq!(base_path(&ingress(&format!("/{}", "a".repeat(300)))), "/");
    }

    #[test]
    fn inject_base_opens_head() {
        let html = inject_base(INDEX, "/p/");

        assert!(html.starts_with(r#"<!DOCTYPE html><html><head><base href="/p/"><script>window.__BIFROST_BASE__="/p/";</script><meta"#), "{html}");
    }

    #[test]
    fn inject_base_handles_head_attributes_and_case() {
        let html = inject_base(
            r#"<HTML><HEAD lang="en"><title>x</title></HEAD></HTML>"#,
            "/",
        );

        assert_eq!(
            html,
            r#"<HTML><HEAD lang="en"><base href="/"><script>window.__BIFROST_BASE__="/";</script><title>x</title></HEAD></HTML>"#
        );
    }

    #[test]
    fn inject_base_skips_header_elements() {
        let html = inject_base("<body><header>x</header></body>", "/");

        assert!(html.starts_with(r#"<base href="/">"#), "{html}");
        assert!(html.ends_with("<body><header>x</header></body>"), "{html}");
    }

    #[tokio::test]
    async fn root_serves_the_shell() {
        let ui = Ui::new();

        let (response, body) = ui.get("/", None).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header(&response, &CONTENT_TYPE), "text/html; charset=utf-8");
        assert_eq!(header(&response, &CACHE_CONTROL), "no-store");
        assert!(body.contains(r#"<base href="/">"#), "{body}");
        assert!(body.contains(r#"window.__BIFROST_BASE__="/";"#), "{body}");
        assert!(body.contains(r#"src="./_nuxt/entry.js""#), "{body}");
    }

    #[tokio::test]
    async fn shell_carries_the_ingress_prefix() {
        let ui = Ui::new();

        let (response, body) = ui.get("/", Some(INGRESS)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            body.contains(&format!(r#"<base href="{INGRESS}/">"#)),
            "{body}"
        );
        assert!(
            body.contains(&format!(r#"window.__BIFROST_BASE__="{INGRESS}/";"#)),
            "{body}"
        );
    }

    #[tokio::test]
    async fn unknown_page_paths_serve_the_shell() {
        let ui = Ui::new();

        for path in [
            "/index.html",
            "/devices",
            "/settings/auth",
            "/apiary",
            "/clipboard",
        ] {
            let (response, body) = ui.get(path, None).await;

            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert!(body.contains(r#"<base href="/">"#), "{path}: {body}");
        }
    }

    #[tokio::test]
    async fn assets_are_served_with_their_type() {
        let ui = Ui::new();

        let (response, body) = ui.get("/_nuxt/entry.js", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header(&response, &CONTENT_TYPE), "text/javascript");
        assert_eq!(
            header(&response, &CACHE_CONTROL),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(body, "console.log(1)");

        let (response, _) = ui.get("/robots.txt", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header(&response, &CACHE_CONTROL), "no-cache");
    }

    #[tokio::test]
    async fn missing_files_are_404_not_the_shell() {
        let ui = Ui::new();

        for path in ["/_nuxt/gone.js", "/favicon.ico", "/deep/style.css"] {
            let (response, body) = ui.get(path, None).await;

            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(body, "", "{path}");
        }
    }

    #[tokio::test]
    async fn paths_cannot_leave_the_ui_directory() {
        let ui = Ui::new();

        for path in ["/../secret.txt", "/_nuxt/../../secret.txt", "//secret.txt"] {
            let (response, body) = ui.get(path, None).await;

            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert!(!body.contains("do not serve"), "{path}: {body}");
        }
    }

    #[tokio::test]
    async fn api_prefixes_never_get_the_shell() {
        let ui = Ui::new();

        for path in [
            "/api",
            "/api/x/y/z/w/v",
            "/auth/v2",
            "/bifrost/nothing",
            "/clip/v2/x",
            "/description.xml",
            "/eventstream/x",
            "/licenses/x",
            "/updater/x",
        ] {
            let (response, body) = ui.get(path, None).await;

            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(header(&response, &CONTENT_TYPE), "", "{path}");
            assert_eq!(body, "", "{path}");
        }
    }

    #[tokio::test]
    async fn only_reads_are_answered() {
        let ui = Ui::new();

        let (response, _) = ui.request(&Method::HEAD, "/", None).await;
        assert_eq!(response.status(), StatusCode::OK);

        for method in [Method::POST, Method::PUT, Method::DELETE] {
            let (response, body) = ui.request(&method, "/", None).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method}");
            assert_eq!(body, "", "{method}");
        }
    }

    #[tokio::test]
    async fn unbuilt_ui_answers_plain_text() {
        let dir = TempDir::new();

        let (response, body) = split(serve(
            UiSource::Directory(dir.path()),
            &Method::GET,
            "/",
            &HeaderMap::new(),
        ))
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            header(&response, &CONTENT_TYPE),
            "text/plain; charset=utf-8"
        );
        assert!(!body.contains('<'), "{body}");
    }

    /// Through the real router, with a built UI: the fallback serves the
    /// shell, unknown Hue and admin paths stay non-HTML, and the Hue v1
    /// catch-all keeps answering.
    #[tokio::test]
    async fn router_keeps_api_paths_off_the_shell() {
        // the only test that reads the process-wide source; set once per run
        super::serve_from_directory(Utf8PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/w02/ui"
        )));
        let dir = TempDir::new();
        let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();
        let router = routes::router(state, hue_cors_layer());

        let call = |path: &'static str| {
            let router = router.clone();
            async move {
                // the real listener always supplies the peer, and /bifrost needs it
                let mut request = Request::builder()
                    .uri(path)
                    .header("host", "10.12.0.20")
                    .body(Body::empty())
                    .unwrap();
                request
                    .extensions_mut()
                    .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));
                split(router.oneshot(request).await.unwrap()).await
            }
        };

        for path in ["/", "/devices"] {
            let (response, body) = call(path).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert!(body.contains(r#"<base href="/">"#), "{path}: {body}");
        }

        for path in [
            "/clip/v2/x",
            "/clip/v2/resource/light/x/y",
            "/api/x/y/z/w/v",
            "/eventstream/x",
            "/updater/x",
        ] {
            let (response, body) = call(path).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert!(!header(&response, &CONTENT_TYPE).contains("html"), "{path}");
            assert!(!body.contains('<'), "{path}: {body}");
        }

        // the admin guard answers before any route lookup: no session, no shell
        let (response, body) = call("/bifrost/nothing/here").await;
        assert!(response.status().is_client_error(), "{}", response.status());
        assert!(!header(&response, &CONTENT_TYPE).contains("html"));
        assert!(!body.contains('<'), "{body}");

        // a single segment is a Hue v1 username: the bridge answers JSON, as before
        let (response, _) = call("/api/x").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header(&response, &CONTENT_TYPE), "application/json");
    }
}
