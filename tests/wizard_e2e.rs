//! A fresh install, end to end: no configuration file, the wizard's own
//! requests, and a light the Hue v1 API answers with afterwards.
//!
//! Setup mode, the auth session, the draft and the apply are the real
//! handlers; the z2m server is the mock and the core bridge services are
//! stood in for, so the test binds no privileged port.

mod support;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Method, Request};
use hyper::StatusCode;
use serde_json::{Value, json};
use svc::manager::ServiceManager;
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};
use tower::ServiceExt;
use tower_http::cors::CorsLayer;

use bifrost::auth::{Auth, HashCost, SystemClock};
use bifrost::config::AppConfig;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

const PATIENCE: Duration = Duration::from_secs(10);
const PASSWORD: &str = "correct horse battery staple";
const LISTEN: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
const OWNER: std::net::Ipv4Addr = std::net::Ipv4Addr::LOCALHOST;

struct CoreIdleFactory {
    z2m: AppServiceFactory,
}

impl ServiceFactory for CoreIdleFactory {
    fn build(&self, _name: &str, _config: &AppConfig) -> ApiResult<BoxDynService> {
        Ok(Idle.boxed())
    }

    fn z2m_template(&self) -> Box<dyn ServiceTemplate> {
        self.z2m.z2m_template()
    }
}

struct Idle;

#[async_trait::async_trait]
impl Service for Idle {
    type Error = std::convert::Infallible;

    async fn run(&mut self) -> Result<(), std::convert::Infallible> {
        futures::future::pending().await
    }
}

/// Send one request to `router` as a browser on the bridge's own address.
async fn call(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    session: Option<(&str, &str)>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1")
        .header("origin", "http://127.0.0.1")
        .header("content-type", "application/json");
    if let Some((cookie, csrf)) = session {
        request = request
            .header("cookie", cookie)
            .header("x-bifrost-csrf", csrf);
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let mut request = request.body(body).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(IpAddr::V4(OWNER), 50_000)));

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let mut value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    if let (Some(cookie), Value::Object(map)) = (cookie, &mut value) {
        map.insert("__cookie".into(), json!(cookie));
    }
    (status, value)
}

#[tokio::test]
async fn a_fresh_install_becomes_a_light_the_v1_api_reports() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let dir = TempDir::new();
    let config_file = dir.path().join("config.yaml");

    // nothing written yet: this is what a fresh install looks like
    assert!(!config_file.exists());

    let auth = Arc::new(Auth::load(
        &dir.path().join("sessions.yaml"),
        Arc::new(SystemClock),
        HashCost::DEFAULT,
    ));
    let state = bifrost::routes::bifrost::setup::SetupState::new(
        &config_file,
        "missing".into(),
        LISTEN,
        auth.clone(),
    );
    let setup = bifrost::routes::bifrost::setup::router(state);
    let code = auth.issue_setup_code();

    // the password comes first and gates everything
    let (status, session) = call(
        &setup,
        Method::POST,
        "/bifrost/auth/setup",
        Some(json!({"password": PASSWORD, "setup_code": code})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let cookie = session["__cookie"].as_str().unwrap_or_default().to_string();
    let cookie = cookie.split(';').next().unwrap_or_default().to_string();
    let csrf = session["csrf"].as_str().unwrap().to_string();

    // the wizard's review: one draft write, then apply
    let document = json!({
        "bridge": {
            "name": "Bifrost",
            "mac": "00:11:22:33:44:55",
            "ipaddress": "127.0.0.1",
            "netmask": "255.0.0.0",
            "gateway": "127.0.0.1",
            "timezone": "UTC"
        },
        "z2m": {"server1": {"url": mock.url()}},
        "devices": {
            "0x0017880100000001": {"enabled": true},
            "0x0017880100000002": {"enabled": true},
            "0x0017880100000003": {"enabled": true}
        },
        "rooms": {"Kitchen": {"devices": ["0x0017880100000001", "0x0017880100000002"]}}
    });

    let (status, draft) = call(
        &setup,
        Method::PUT,
        "/bifrost/config/draft",
        Some(json!({"revision": 0, "config": document})),
        Some((&cookie, &csrf)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    let revision = draft["revision"].as_u64().unwrap();

    let (status, applied) = call(
        &setup,
        Method::POST,
        "/bifrost/config/apply",
        Some(json!({"revision": revision})),
        Some((&cookie, &csrf)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{applied}");

    // the configuration file exists now, and the bridge can start from it
    assert!(config_file.exists(), "setup wrote no configuration");

    let store = Arc::new(ConfigStore::load(&config_file).unwrap());
    let (mut svm, _future) = ServiceManager::spawn();
    let appstate = AppState::from_store(store.clone(), svm.clone())
        .await
        .unwrap();

    let factory = CoreIdleFactory {
        z2m: AppServiceFactory::new(appstate.clone()),
    };
    service_registry::reconcile(&store, &mut svm, &factory, &[])
        .await
        .unwrap();

    svm.wait_for_start("z2m@server1").await.unwrap();

    // the backend imports what the mock announces
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        if appstate.inventory().snapshot().devices.len() >= 3 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the devices were never imported"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // and the Hue v1 API answers with a light
    let hue: Router = bifrost::routes::router(appstate.clone(), CorsLayer::permissive());
    let (status, lights) = call(&hue, Method::GET, "/api/testuser/lights", None, None).await;

    assert_eq!(status, StatusCode::OK, "{lights}");
    let text = lights.to_string();
    assert!(text.contains("Kitchen ceiling"), "{text}");
    assert!(text.contains("Hall lamp"), "{text}");
    // the attic bulb is in no group, so it is not in the room
    assert!(text.contains("Attic bulb"), "{text}");

    // the room the wizard drafted is what the v2 API answers with
    let (status, rooms) = call(&hue, Method::GET, "/clip/v2/resource/room", None, None).await;
    assert_eq!(status, StatusCode::OK, "{rooms}");
    let rooms = rooms.to_string();
    assert!(rooms.contains("Kitchen"), "{rooms}");
}
