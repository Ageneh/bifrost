//! Which area a v1 stream belongs to, and where a zone's channels sit.
//!
//! A v1 client names lights, not an area, so the bridge has to infer one. The
//! rule has to be deterministic the moment two zones share a light, which is
//! the case this wave's editor makes easy to build by accident.
//!
//! The second half is the positions an editor saves: they must come back from
//! the v2 `GET` unchanged, and a zone that never got positions must still list
//! its channels somewhere sensible.

mod support;

use std::collections::BTreeSet;
use std::net::SocketAddr;
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
use uuid::Uuid;

use bifrost::config::AppConfig;
use bifrost::config_store::ConfigStore;
use bifrost::error::{ApiError, ApiResult};
use bifrost::routes;
use bifrost::routes::clip::entertainment_configuration::auto_layout;
use bifrost::server::appstate::AppState;
use bifrost::server::entertainment::Session;
use bifrost::server::hue_cors_layer;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use hue::api::{
    Entertainment, EntertainmentConfigurationStatus, EntertainmentConfigurationType, Position,
    RType, ResourceLink,
};
use support::entertainment::add_area;
use support::mock_z2m::{MockZ2m, fixture_path};
use support::temp_dir::TempDir;

const LIGHT_A: &str = "0x00178801000000a1";
const LIGHT_B: &str = "0x00178801000000b1";

const BRIDGE_HOST: &str = "127.0.0.1";
const CLIENT: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 2));

const PATIENCE: Duration = Duration::from_secs(10);

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

    async fn wait_for_lights(&self) {
        let deadline = tokio::time::Instant::now() + PATIENCE;
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
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn light(&self, ieee: &str) -> ResourceLink {
        let device = self
            .state
            .inventory()
            .devices_by_ieee(ieee)
            .into_iter()
            .next()
            .expect("no such device");

        let resource = device
            .resources
            .iter()
            .find(|resource| resource.rtype == "light")
            .expect("no light resource");

        RType::Light.link_to(resource.rid.parse().expect("a resource id"))
    }

    async fn entertainment(&self, light: ResourceLink) -> ResourceLink {
        let res = self.state.res.lock().await;
        res.get_resource_ids_by_type(RType::Entertainment)
            .into_iter()
            .find(|id| {
                res.get_id::<Entertainment>(*id)
                    .is_ok_and(|ent| ent.renderer_reference == Some(light))
            })
            .map(|id| RType::Entertainment.link_to(id))
            .expect("no entertainment service")
    }

    /// An area over `lights`, streaming or not.
    async fn area(&self, name: &str, lights: &[ResourceLink], streaming: bool) -> Uuid {
        let mut res = self.state.res.lock().await;
        let area = add_area(&mut res, name, lights);
        if streaming {
            res.update::<hue::api::EntertainmentConfiguration>(&area, |ec| {
                ec.status = EntertainmentConfigurationStatus::Active;
            })
            .unwrap();
        }
        drop(res);
        area
    }

    /// The area a v1 frame naming `lights` would be served by.
    #[allow(clippy::result_large_err)]
    async fn select(&self, lights: &[ResourceLink]) -> ApiResult<Uuid> {
        let ids: BTreeSet<Uuid> = lights.iter().map(|link| link.rid).collect();
        let res = self.state.res.lock().await;
        Session::select_area(&res, &ids)
    }

    async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("host", BRIDGE_HOST)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(CLIENT, 50000)));

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn put(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(Method::PUT)
            .uri(uri)
            .header("host", BRIDGE_HOST)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(CLIENT, 50000)));

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header("host", BRIDGE_HOST)
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(CLIENT, 50000)));

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Create a zone through the v2 `POST` the Hue app uses, and return its id.
    async fn create_zone(&self, name: &str, services: &[(ResourceLink, Vec<Position>)]) -> Uuid {
        let body = json!({
            "configuration_type": "screen",
            "metadata": {"name": name},
            "stream_proxy": null,
            "locations": {
                "service_locations": services.iter().map(|(service, positions)| json!({
                    "positions": positions,
                    "service": {"rid": service.rid, "rtype": "entertainment"},
                })).collect::<Vec<_>>(),
            },
        });

        let (status, reply) = self
            .post("/clip/v2/resource/entertainment_configuration", body)
            .await;
        assert_eq!(status, StatusCode::OK, "the zone was not created: {reply}");
        reply["data"][0]["rid"]
            .as_str()
            .expect("no zone id")
            .parse()
            .expect("a zone id")
    }
}

/// An area that is already streaming keeps the frame, even when another area
/// claims more of its lights.
#[tokio::test]
async fn the_streaming_area_wins() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let light_a = fixture.light(LIGHT_A);
    let light_b = fixture.light(LIGHT_B);
    let ent_a = fixture.entertainment(light_a).await;
    let ent_b = fixture.entertainment(light_b).await;

    // both areas claim light A; the one with more of the frame's lights is
    // the second, until the first says it is streaming
    let streaming = fixture.area("TV", &[ent_a], true).await;
    let _wider = fixture.area("Desk", &[ent_a, ent_b], false).await;

    let selected = fixture
        .select(&[light_a, light_b])
        .await
        .expect("the frame was refused");
    assert_eq!(
        selected, streaming,
        "the streaming area did not keep the frame"
    );
}

/// Without a streaming area the unique largest intersection wins.
#[tokio::test]
async fn the_unique_largest_intersection_wins() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let light_a = fixture.light(LIGHT_A);
    let light_b = fixture.light(LIGHT_B);
    let ent_a = fixture.entertainment(light_a).await;
    let ent_b = fixture.entertainment(light_b).await;

    let tv = fixture.area("TV", &[ent_a, ent_b], false).await;
    let _desk = fixture.area("Desk", &[ent_a], false).await;

    let selected = fixture
        .select(&[light_a, light_b])
        .await
        .expect("the frame was refused");
    assert_eq!(selected, tv, "the area claiming both lights did not win");
}

/// Two areas claiming the same lights are refused, and the refusal names both
/// so a person can act on it.
#[tokio::test]
async fn a_tie_is_refused_and_names_the_areas() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let light_a = fixture.light(LIGHT_A);
    let light_b = fixture.light(LIGHT_B);
    let ent_a = fixture.entertainment(light_a).await;
    let ent_b = fixture.entertainment(light_b).await;

    fixture.area("TV", &[ent_a], false).await;
    fixture.area("Desk", &[ent_b], false).await;

    let error = fixture
        .select(&[light_a, light_b])
        .await
        .expect_err("a tie was guessed instead of refused");

    let ApiError::EntStreamAmbiguous(names) = error else {
        panic!("the refusal does not name the areas: {error:?}");
    };
    assert!(
        names.contains("TV"),
        "the refusal lost the first name: {names}"
    );
    assert!(
        names.contains("Desk"),
        "the refusal lost the second name: {names}"
    );
}

/// An area with none of the frame's lights cannot render it.
#[tokio::test]
async fn an_area_without_the_lights_is_not_selected() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let light_a = fixture.light(LIGHT_A);
    let light_b = fixture.light(LIGHT_B);
    let ent_b = fixture.entertainment(light_b).await;

    fixture.area("Desk", &[ent_b], false).await;

    let error = fixture
        .select(&[light_a])
        .await
        .expect_err("an unrelated area was selected");
    assert!(
        matches!(error, ApiError::EntStreamInitError),
        "the refusal was not the init error: {error:?}"
    );
}

/// The editor's positions are the channel positions, and the v2 `GET` returns
/// them unchanged.
#[tokio::test]
async fn a_screen_zone_lists_its_channels_where_the_editor_put_them() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let ent_b = fixture.entertainment(fixture.light(LIGHT_B)).await;

    let left = Position::new(-0.4, 0.8, 0.4);
    let right = Position::new(0.4, 0.8, 0.4);
    let area = fixture
        .create_zone(
            "TV",
            &[(ent_a, vec![left.clone()]), (ent_b, vec![right.clone()])],
        )
        .await;

    let (status, body) = fixture
        .get(&format!(
            "/clip/v2/resource/entertainment_configuration/{area}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let zone = &body["data"][0];
    assert_eq!(zone["configuration_type"], "screen", "{body}");

    let saved = zone["locations"]["service_locations"]
        .as_array()
        .expect("no locations");
    assert_eq!(saved.len(), 2, "{body}");
    assert_eq!(saved[0]["positions"][0]["x"], -0.4, "{body}");
    assert_eq!(saved[1]["positions"][0]["x"], 0.4, "{body}");

    let channels = zone["channels"].as_array().expect("no channels");
    assert_eq!(channels.len(), 2, "{body}");
    assert_eq!(channels[0]["position"]["x"], -0.4, "{body}");
    assert_eq!(channels[1]["position"]["x"], 0.4, "{body}");
}

/// A `PUT` replaces the positions, and the next `GET` returns exactly what
/// was written.
#[tokio::test]
async fn a_saved_position_round_trips_unchanged() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let area = fixture
        .create_zone("TV", &[(ent_a, vec![Position::new(0.0, 0.0, 0.0)])])
        .await;

    let moved = Position::new(0.125, 0.5, 0.75);
    let (status, reply) = fixture
        .put(
            &format!("/clip/v2/resource/entertainment_configuration/{area}"),
            json!({
                "locations": {
                    "service_locations": [
                        {"positions": [{"x": moved.x, "y": moved.y, "z": moved.z}], "service": {"rid": ent_a.rid, "rtype": "entertainment"}},
                    ],
                },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reply}");

    let (status, body) = fixture
        .get(&format!(
            "/clip/v2/resource/entertainment_configuration/{area}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let saved = &body["data"][0]["locations"]["service_locations"][0]["positions"][0];
    assert_eq!(saved["x"], moved.x, "{body}");
    assert_eq!(saved["y"], moved.y, "{body}");
    assert_eq!(saved["z"], moved.z, "{body}");

    let channel = &body["data"][0]["channels"][0]["position"];
    assert_eq!(channel["x"], moved.x, "the channel did not follow: {body}");
}

/// A zone the editor never laid out still lists its channels, on the default
/// layout.
#[tokio::test]
async fn a_zone_without_positions_gets_the_auto_layout() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let area = fixture.create_zone("TV", &[(ent_a, vec![])]).await;

    let (status, body) = fixture
        .get(&format!(
            "/clip/v2/resource/entertainment_configuration/{area}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let default = auto_layout(1);
    let channel = &body["data"][0]["channels"][0]["position"];
    assert_eq!(channel["x"], default[0].x, "{body}");
    assert_eq!(channel["y"], default[0].y, "{body}");
    assert_eq!(channel["z"], default[0].z, "{body}");
}

/// The default layout depends on nothing but the segment count, so the editor
/// can compute the same one before anything is saved.
#[test]
#[allow(clippy::float_cmp)]
fn the_auto_layout_is_byte_identical_for_identical_input() {
    let once = serde_json::to_string(&auto_layout(7)).unwrap();
    let twice = serde_json::to_string(&auto_layout(7)).unwrap();
    assert_eq!(once, twice, "the default layout is not deterministic");

    let points = auto_layout(7);
    assert_eq!(points.len(), 7);
    assert_eq!(points[0].x, -0.4);
    assert_eq!(points[6].x, 0.4);
    assert_eq!(points[3].x, 0.0, "seven points have no centre: {points:?}");
    assert!(points.iter().all(|p| p.y == 0.8 && p.z == 0.4));

    assert_eq!(auto_layout(0).len(), 0);
    assert_eq!(auto_layout(1), vec![Position::new(0.0, 0.8, 0.4)]);
}

/// A zone created with the type the Hue app sends for a TV keeps it.
#[tokio::test]
async fn a_tv_zone_is_a_screen_zone() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let area = fixture
        .create_zone("TV", &[(ent_a, vec![Position::new(0.0, 0.8, 0.4)])])
        .await;

    let kind = {
        let res = fixture.state.res.lock().await;
        let ec = res
            .get_id::<hue::api::EntertainmentConfiguration>(area)
            .unwrap();
        let kind = ec.configuration_type.clone();
        drop(res);
        kind
    };
    assert_eq!(
        kind,
        EntertainmentConfigurationType::Screen,
        "the zone is not a screen"
    );
}
