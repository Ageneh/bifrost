//! The entertainment endpoints, over the real admin router.
//!
//! A real z2m backend imports the devices from a mock server, so the
//! capabilities answer is built from resources the import created, not from
//! ones the test wrote by hand.

mod support;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Method, Request};
use hyper::StatusCode;
use serde_json::Value;
use svc::manager::ServiceManager;
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};
use tower::ServiceExt;

use bifrost::auth::Expiry;
use bifrost::config::AppConfig;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::routes;
use bifrost::server::appstate::AppState;
use bifrost::server::hue_cors_layer;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use support::entertainment::add_area;
use support::mock_z2m::{MockZ2m, fixture_path};
use support::temp_dir::TempDir;

const LIGHT_A: &str = "0x00178801000000a1";
const LIGHT_B: &str = "0x00178801000000b1";

/// The bridge address the fixture config declares, which the guard checks.
const BRIDGE_HOST: &str = "127.0.0.1";

/// The address the requests come from.
const CLIENT: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 2));

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

struct Fixture {
    _dir: TempDir,
    state: AppState,
    router: Router,
}

impl Fixture {
    async fn start(mock: &MockZ2m) -> Self {
        let dir = TempDir::new();
        let yaml = format!(
            "\
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: {BRIDGE_HOST}
  netmask: 255.0.0.0
  gateway: {BRIDGE_HOST}
  timezone: UTC

z2m:
  server1:
    url: {}
",
            mock.url()
        );
        let store = Arc::new(ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap());
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(store.clone(), svm.clone())
            .await
            .unwrap();

        let factory = CoreIdleFactory {
            z2m: AppServiceFactory::new(state.clone()),
        };
        service_registry::reconcile(&store, &mut { svm }, &factory, &[])
            .await
            .unwrap();

        let router = routes::router(state.clone(), hue_cors_layer());

        Self {
            _dir: dir,
            state,
            router,
        }
    }

    /// Wait until the backend has imported both lights.
    async fn wait_for_lights(&self) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let ready = [LIGHT_A, LIGHT_B].iter().all(|ieee| {
                self.state
                    .inventory()
                    .devices_by_ieee(ieee)
                    .first()
                    .is_some_and(|device| device.resources.iter().any(|r| r.rtype == "light"))
            });
            if ready {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the backend did not import its lights"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    /// POST `uri` with a fresh session and its CSRF token.
    async fn post(&self, uri: &str) -> (StatusCode, Value) {
        let session = self.session();
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("host", BRIDGE_HOST)
            .header("origin", format!("http://{BRIDGE_HOST}"))
            .header("cookie", format!("bifrost_session={}", session.token))
            .header("x-bifrost-csrf", &session.csrf)
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(CLIENT, 50000)));

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        (status, body)
    }

    fn session(&self) -> bifrost::auth::NewSession {
        self.state
            .auth()
            .create_session(
                CLIENT,
                Expiry::from_config(&self.state.config().bifrost.auth),
            )
            .unwrap()
    }

    /// GET `uri` with a fresh session.
    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let session = self
            .state
            .auth()
            .create_session(
                CLIENT,
                Expiry::from_config(&self.state.config().bifrost.auth),
            )
            .unwrap();

        let mut request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header("host", BRIDGE_HOST)
            .header("cookie", format!("bifrost_session={}", session.token))
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(CLIENT, 50000)));

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        (status, body)
    }
}

/// The capabilities answer names the devices that can render, the zones that
/// exist, and the limit the configuration sets.
#[tokio::test]
async fn capabilities_lists_devices_zones_and_the_limit() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    {
        let mut res = fixture.state.res.lock().await;
        let ent_a = res
            .get_resource_ids_by_type(hue::api::RType::Entertainment)
            .into_iter()
            .find(|id| {
                res.get_id::<hue::api::Entertainment>(*id)
                    .is_ok_and(|ent| ent.renderer_reference.is_some())
            })
            .expect("no entertainment service was imported");
        add_area(
            &mut res,
            "TV",
            &[hue::api::RType::Entertainment.link_to(ent_a)],
        );
        drop(res);
    }

    let (status, body) = fixture.get("/bifrost/entertainment/capabilities").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["max_streams"], 2, "{body}");

    let devices = body["devices"].as_array().unwrap();
    assert_eq!(
        devices.len(),
        2,
        "every light renders entertainment: {body}"
    );
    assert_eq!(devices[0]["name"], "TV left", "{body}");
    assert_eq!(devices[1]["name"], "TV right", "{body}");
    assert_eq!(devices[0]["segments"], 1, "{body}");

    let configurations = body["configurations"].as_array().unwrap();
    assert_eq!(configurations.len(), 1, "{body}");
    assert_eq!(configurations[0]["name"], "TV", "{body}");
    assert_eq!(configurations[0]["configuration_type"], "screen", "{body}");
    assert_eq!(configurations[0]["status"], "inactive", "{body}");
    assert_eq!(configurations[0]["channels"], 1, "{body}");
}

/// The status answer carries the limit and one row per stream; with nothing
/// streaming it is an empty list, not an error.
#[tokio::test]
async fn status_reports_the_limit_and_no_streams_when_idle() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;

    let (status, body) = fixture.get("/bifrost/entertainment/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["max_streams"], 2, "{body}");
    assert_eq!(body["streams"], serde_json::json!([]), "{body}");
}

/// Stopping something that is not streaming is a 404, not a crash, and the
/// area id has to be a uuid.
#[tokio::test]
async fn stop_answers_not_found_for_anything_that_is_not_streaming() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    let area = uuid::Uuid::new_v4();

    for uri in [
        format!("/bifrost/entertainment/{area}/stop"),
        "/bifrost/entertainment/not-a-uuid/stop".to_string(),
    ] {
        let (status, body) = fixture.post(&uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }
}

/// One zone, as the editor reads it: its positions, and the channels derived
/// from them.
#[tokio::test]
async fn a_zone_is_read_with_its_positions_and_channels() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let area = {
        let mut res = fixture.state.res.lock().await;
        let ent_a = res
            .get_resource_ids_by_type(hue::api::RType::Entertainment)
            .into_iter()
            .find(|id| {
                res.get_id::<hue::api::Entertainment>(*id)
                    .is_ok_and(|ent| ent.renderer_reference.is_some())
            })
            .expect("no entertainment service was imported");

        let link = hue::api::RType::Entertainment.link_to(ent_a);
        support::entertainment::add_area(&mut res, "TV", &[link])
    };

    let (status, body) = fixture
        .get(&format!("/bifrost/entertainment/configurations/{area}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["id"], area.to_string(), "{body}");
    assert_eq!(body["name"], "TV", "{body}");
    assert_eq!(body["configuration_type"], "screen", "{body}");
    assert_eq!(body["status"], "inactive", "{body}");
    assert_eq!(body["locations"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["channels"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["channels"][0]["channel_id"], 0, "{body}");
}

/// A zone that does not exist, or an id that is not one, is a 404 rather than
/// an error page.
#[tokio::test]
async fn an_unknown_zone_is_not_found() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;

    for id in [uuid::Uuid::new_v4().to_string(), "not-a-uuid".to_string()] {
        let (status, body) = fixture
            .get(&format!("/bifrost/entertainment/configurations/{id}"))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}: {body}");
    }
}

/// The endpoint needs a session like every other read under `/bifrost`.
#[tokio::test]
async fn capabilities_needs_a_session() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;

    let mut request = Request::builder()
        .method(Method::GET)
        .uri("/bifrost/entertainment/capabilities")
        .header("host", BRIDGE_HOST)
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(CLIENT, 50000)));

    let response = fixture.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
