//! Area routing against two real z2m backends and two mock servers.
//!
//! The backend, the service registry and the resource store are the real
//! ones; only the core bridge services are stood in for, so the test binds no
//! privileged port.

mod support;

use std::sync::Arc;
use std::time::Duration;

use svc::manager::{ServiceManager, SvmClient};
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};

use bifrost::config::AppConfig;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use bifrost_api::backend::BackendRequest;
use hue::api::{
    Entertainment, EntertainmentConfiguration, EntertainmentConfigurationStatus, Light, LightMode,
    LightUpdate, RType, ResourceLink,
};
use support::entertainment::{add_area, frame};
use support::mock_z2m::{MockZ2m, fixture_path};
use support::temp_dir::TempDir;

/// The two lights the fixtures announce, one per server.
const LIGHT_A: &str = "0x00178801000000a1";
const LIGHT_B: &str = "0x00178801000000b1";

const PATIENCE: Duration = Duration::from_secs(10);

/// Stands in for the core services (http, https, mdns, ssdp, entertainment),
/// which would bind real ports; z2m backends are the real thing.
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
    store: Arc<ConfigStore>,
    svm: SvmClient,
    state: AppState,
}

impl Fixture {
    /// Start Bifrost against the two mock servers, `a` and `b`.
    async fn start(a: &MockZ2m, b: &MockZ2m) -> Self {
        Self::start_with(&[("serverA", a.url()), ("serverB", b.url())]).await
    }

    /// Start Bifrost against the named z2m servers.
    async fn start_with(servers: &[(&str, &str)]) -> Self {
        let dir = TempDir::new();
        let servers: String = servers
            .iter()
            .map(|(name, url)| format!("  {name}:\n    url: {url}\n"))
            .collect::<String>();
        let yaml = format!(
            "\
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 127.0.0.1
  netmask: 255.0.0.0
  gateway: 127.0.0.1
  timezone: UTC

z2m:
{servers}"
        );
        let store = Arc::new(ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap());
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(store.clone(), svm.clone())
            .await
            .unwrap();

        let mut fixture = Self {
            _dir: dir,
            store,
            svm,
            state,
        };
        fixture.reconcile().await;
        fixture
    }

    async fn reconcile(&mut self) {
        let factory = CoreIdleFactory {
            z2m: AppServiceFactory::new(self.state.clone()),
        };
        service_registry::reconcile(&self.store, &mut self.svm, &factory, &[])
            .await
            .unwrap();
    }

    /// Wait until both backends have imported their light.
    async fn wait_for_lights(&self) {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let ready = [LIGHT_A, LIGHT_B].iter().all(|ieee| {
                let devices = self.state.inventory().devices_by_ieee(ieee);
                devices
                    .first()
                    .is_some_and(|device| device.resources.iter().any(|r| r.rtype == "light"))
            });
            if ready {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the backends did not import their lights within {PATIENCE:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The light service of the light `ieee`, as the import built it.
    ///
    /// The rid comes from the inventory, never recomputed here, so the test
    /// cannot drift from the backend.
    fn light(&self, ieee: &str) -> ResourceLink {
        let device = self
            .state
            .inventory()
            .devices_by_ieee(ieee)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("no device {ieee} in the inventory"));

        let resource = device
            .resources
            .iter()
            .find(|resource| resource.rtype == "light")
            .unwrap_or_else(|| panic!("no light for {ieee}: {device:?}"));

        RType::Light.link_to(resource.rid.parse().expect("a resource id"))
    }

    /// The entertainment service that renders `light`.
    async fn entertainment(&self, light: ResourceLink) -> ResourceLink {
        let res = self.state.res.lock().await;
        res.get_resource_ids_by_type(RType::Entertainment)
            .into_iter()
            .find(|id| {
                res.get_id::<Entertainment>(*id)
                    .is_ok_and(|ent| ent.renderer_reference == Some(light))
            })
            .map_or_else(
                || panic!("no entertainment service renders {light:?}"),
                |id| RType::Entertainment.link_to(id),
            )
    }

    async fn request(&self, req: BackendRequest) {
        self.state.res.lock().await.backend_request(req).unwrap();
    }
}

/// A light update addressed to `link`, as a room or scene write would
/// produce.
fn turn_on(link: ResourceLink) -> BackendRequest {
    BackendRequest::LightUpdate(
        link,
        LightUpdate {
            on: Some(hue::api::On { on: true }),
            ..LightUpdate::default()
        },
    )
}

/// The single stream that owns `LIGHT_A` must not disturb the server that
/// owns nothing in it, and that server's event loop must survive the request.
///
/// This is the case the reviewer called already broken: every backend
/// receives every request, and a backend that did not own the area's lights
/// failed on the lookup, which ended its event loop and with it every light
/// on that server.
#[tokio::test]
async fn a_server_that_owns_none_of_the_area_ignores_it_and_keeps_running() {
    let a = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w07/z2m/server-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let area = {
        let mut res = fixture.state.res.lock().await;
        add_area(&mut res, "TV", &[ent_a])
    };

    fixture
        .request(BackendRequest::EntertainmentStart(area))
        .await;

    // the owner acknowledges the start by putting the device into
    // entertainment mode; the other server must send nothing at all
    wait_for_count(&a, "TV left/set", 1).await;
    assert!(
        b.received().is_empty(),
        "the server that owns nothing in the area spoke: {:?}",
        b.received()
    );

    // and its event loop is still there: a plain light update still arrives
    fixture.request(turn_on(fixture.light(LIGHT_B))).await;
    wait_for_count(&b, "TV right/set", 1).await;
}

/// A frame is addressed to one area and reaches only the server that streams
/// it, and a stop names the area too.
#[tokio::test]
async fn frames_reach_only_the_area_that_owns_them() {
    let a = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w07/z2m/server-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let area = {
        let mut res = fixture.state.res.lock().await;
        add_area(&mut res, "TV", &[ent_a])
    };

    fixture
        .request(BackendRequest::EntertainmentStart(area))
        .await;
    a.wait_for("TV left/set").await;

    // the start itself wrote once; every further write is a forwarded frame
    let before = a.received_on("TV left/set").len();
    for _ in 0..5 {
        fixture
            .request(BackendRequest::EntertainmentFrame(area, frame(1)))
            .await;
        // the backend throttles to its configured rate; a sleep between
        // frames keeps this deterministic without making the test slow
        tokio::time::sleep(Duration::from_millis(60)).await;
    }

    wait_for_count(&a, "TV left/set", before + 1).await;

    assert!(
        b.received().is_empty(),
        "the other server received the stream: {:?}",
        b.received()
    );
}

/// The area a stop names is the area that stops: a second area keeps
/// streaming.
#[tokio::test]
async fn a_stop_names_the_area_it_ends() {
    let a = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w07/z2m/server-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let ent_b = fixture.entertainment(fixture.light(LIGHT_B)).await;
    let (area_a, area_b) = {
        let mut res = fixture.state.res.lock().await;
        (
            add_area(&mut res, "TV", &[ent_a]),
            add_area(&mut res, "Desk", &[ent_b]),
        )
    };

    fixture
        .request(BackendRequest::EntertainmentStart(area_a))
        .await;
    fixture
        .request(BackendRequest::EntertainmentStart(area_b))
        .await;
    wait_for_count(&a, "TV left/set", 1).await;
    wait_for_count(&b, "TV right/set", 1).await;

    fixture
        .request(BackendRequest::EntertainmentStop(area_a))
        .await;

    // the second area still takes frames after the first stopped
    let before = b.received_on("TV right/set").len();
    for _ in 0..5 {
        fixture
            .request(BackendRequest::EntertainmentFrame(area_b, frame(1)))
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }

    wait_for_count(&b, "TV right/set", before + 1).await;
}

/// A frame for an area no backend streams is dropped, not an error.
#[tokio::test]
async fn a_frame_for_an_unknown_area_is_dropped() {
    let a = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w07/z2m/server-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let area = {
        let mut res = fixture.state.res.lock().await;
        add_area(&mut res, "TV", &[ent_a])
    };

    // no start: the frame has nowhere to go, and nothing may break
    fixture
        .request(BackendRequest::EntertainmentFrame(area, frame(1)))
        .await;
    fixture.request(turn_on(fixture.light(LIGHT_A))).await;

    wait_for_count(&a, "TV left/set", 1).await;
}

/// Wait until `mock` has received at least `count` messages on `topic`.
///
/// A frame is throttled to the backend's configured rate, so a test sends
/// frames in a loop rather than assuming every one is forwarded.
async fn wait_for_count(mock: &MockZ2m, topic: &str, count: usize) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while mock.received_on(topic).len() < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no {count} messages on {topic:?}; received {:?}",
            mock.received()
                .iter()
                .map(|message| message.topic.clone())
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The store keeps frames off the control channel, so a control consumer
/// cannot be starved by them.
#[tokio::test]
async fn frames_do_not_travel_on_the_control_channel() {
    let a = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w07/z2m/server-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let area = {
        let mut res = fixture.state.res.lock().await;
        add_area(&mut res, "TV", &[ent_a])
    };

    let mut control = fixture.state.res.lock().await.backend_event_stream();
    let mut frames = fixture.state.res.lock().await.frame_stream();

    fixture
        .request(BackendRequest::EntertainmentFrame(area, frame(1)))
        .await;

    let got = tokio::time::timeout(Duration::from_millis(200), frames.recv())
        .await
        .expect("no frame on the frame channel")
        .expect("the frame channel closed");
    assert!(matches!(&*got, BackendRequest::EntertainmentFrame(..)));

    assert!(
        tokio::time::timeout(Duration::from_millis(100), control.recv())
            .await
            .is_err(),
        "the frame also arrived on the control channel"
    );

    // and a control request does not arrive on the frame channel
    fixture.request(turn_on(fixture.light(LIGHT_A))).await;
    let got = tokio::time::timeout(Duration::from_millis(200), control.recv())
        .await
        .expect("no control request")
        .expect("the control channel closed");
    assert!(matches!(&*got, BackendRequest::LightUpdate(..)));
}

/// Two areas on one server: each keeps its own throttle, and stopping one
/// leaves the other streaming.
#[tokio::test]
async fn two_areas_on_one_server_stream_independently() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-both.json"))
        .await
        .unwrap();
    let fixture = Fixture::start_with(&[("server1", mock.url())]).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let ent_b = fixture.entertainment(fixture.light(LIGHT_B)).await;
    let (area_a, area_b) = {
        let mut res = fixture.state.res.lock().await;
        (
            add_area(&mut res, "TV", &[ent_a]),
            add_area(&mut res, "Desk", &[ent_b]),
        )
    };

    fixture
        .request(BackendRequest::EntertainmentStart(area_a))
        .await;
    fixture
        .request(BackendRequest::EntertainmentStart(area_b))
        .await;
    wait_for_count(&mock, "TV left/set", 1).await;
    wait_for_count(&mock, "TV right/set", 1).await;

    // one frame for each area in quick succession, once each area's own
    // interval has passed. A throttle shared by the backend would spend its
    // one tick on the first and drop the second.
    tokio::time::sleep(Duration::from_millis(60)).await;
    let before_left = mock.received_on("TV left/set").len();
    let before_right = mock.received_on("TV right/set").len();
    fixture
        .request(BackendRequest::EntertainmentFrame(area_a, frame(1)))
        .await;
    fixture
        .request(BackendRequest::EntertainmentFrame(area_b, frame(1)))
        .await;

    wait_for_count(&mock, "TV left/set", before_left + 1).await;
    wait_for_count(&mock, "TV right/set", before_right + 1).await;

    // both areas are streaming, as the v2 API would have recorded it
    {
        let mut res = fixture.state.res.lock().await;
        for id in [fixture.light(LIGHT_A).rid, fixture.light(LIGHT_B).rid] {
            res.update::<Light>(&id, |light| {
                light.mode = LightMode::Streaming;
            })
            .unwrap();
        }
        for (area, name) in [(area_a, "TV"), (area_b, "Desk")] {
            res.update::<EntertainmentConfiguration>(&area, |ec| {
                ec.status = EntertainmentConfigurationStatus::Active;
                ec.metadata.name = name.to_string();
            })
            .unwrap();
        }
    }

    fixture
        .request(BackendRequest::EntertainmentStop(area_a))
        .await;

    // the area that stopped is back to normal; the other one is untouched.
    // the backend handles the stop on its own task, so give it a moment
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while fixture
        .state
        .res
        .lock()
        .await
        .get_id::<Light>(fixture.light(LIGHT_A).rid)
        .unwrap()
        .mode
        == LightMode::Streaming
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the stopped area's light never left streaming mode"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    {
        let res = fixture.state.res.lock().await;

        let light_a = res.get_id::<Light>(fixture.light(LIGHT_A).rid).unwrap();
        let light_b = res.get_id::<Light>(fixture.light(LIGHT_B).rid).unwrap();
        assert_eq!(light_a.mode, LightMode::Normal, "the stopped area's light");
        assert_eq!(light_b.mode, LightMode::Streaming, "the other area's light");

        let ec_a = res.get_id::<EntertainmentConfiguration>(area_a).unwrap();
        let ec_b = res.get_id::<EntertainmentConfiguration>(area_b).unwrap();
        assert!(!ec_a.is_streaming(), "the stopped area is still active");
        assert!(ec_b.is_streaming(), "the other area was stopped with it");
    }

    // and the other area still takes frames
    let before_right = mock.received_on("TV right/set").len();
    for _ in 0..5 {
        fixture
            .request(BackendRequest::EntertainmentFrame(area_b, frame(1)))
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    wait_for_count(&mock, "TV right/set", before_right + 1).await;
}
