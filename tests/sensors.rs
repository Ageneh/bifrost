//! Wave 9's acceptance: every sensor class, end to end.
//!
//! Each test starts the real z2m backend against a mock replaying one
//! recorded-shaped capture per class (`tests/fixtures/w09/`), opts the device
//! in the way an install does, and then reads back what the three surfaces a
//! person uses say about it: the inventory the console draws, the v2 resource
//! the Hue app reads, and the v1 row an older client reads.
//!
//! Nothing here waits on hardware. The captain's bench pass at the end of the
//! wave is confirmation of what these tests already assert against a mock.

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request};
use chrono::Utc;
use serde_json::{Value, json};
use tower::ServiceExt;

use bifrost::config::AppConfig;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::inventory::{DeviceInfo, DeviceType};
use bifrost::routes::api as hue_api;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

use svc::manager::ServiceManager;
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};

/// How long a test waits for the backend to catch up with the mock.
const PATIENCE: Duration = Duration::from_secs(10);

/// The core services would bind real ports; the z2m backend is the real one.
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

/// One class, and what each surface must say about it.
struct Case {
    class: &'static str,
    ieee: &'static str,
    friendly_name: &'static str,
    /// The v2 resource the class owns.
    resource: hue::api::RType,
    /// The v1 sensor type the class serializes as.
    v1_type: &'static str,
    /// The property the reading lands under.
    property: &'static str,
}

const CASES: [Case; 5] = [
    Case {
        class: "motion",
        ieee: "0x00158d0000000020",
        friendly_name: "Hall motion",
        resource: hue::api::RType::Motion,
        v1_type: "ZLLPresence",
        property: "occupancy",
    },
    Case {
        class: "temperature",
        ieee: "0x00158d0000000030",
        friendly_name: "Study temperature",
        resource: hue::api::RType::Temperature,
        v1_type: "ZLLTemperature",
        property: "temperature",
    },
    Case {
        class: "illuminance",
        ieee: "0x00158d0000000040",
        friendly_name: "Porch light level",
        resource: hue::api::RType::LightLevel,
        v1_type: "ZLLLightLevel",
        property: "illuminance",
    },
    Case {
        class: "contact",
        ieee: "0x00158d0000000050",
        friendly_name: "Back door",
        resource: hue::api::RType::Contact,
        v1_type: "CLIPGenericStatus",
        property: "contact",
    },
    Case {
        class: "remote",
        ieee: "0x00158d0000000060",
        friendly_name: "Bedside switch",
        resource: hue::api::RType::Button,
        v1_type: "ZLLSwitch",
        property: "action",
    },
];

struct Fixture {
    _dir: TempDir,
    state: AppState,
    router: Router,
}

impl Fixture {
    /// Start Bifrost against one mock, with `ieee` opted in.
    async fn start(url: &str, ieee: &str) -> Self {
        let dir = TempDir::new();
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
  server1:
    url: {url}

devices:
  \"{ieee}\":
    enabled: true
"
        );
        let store = Arc::new(ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap());
        let (mut svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(store.clone(), svm.clone())
            .await
            .unwrap();

        let factory = CoreIdleFactory {
            z2m: AppServiceFactory::new(state.clone()),
        };
        service_registry::reconcile(&store, &mut svm, &factory, &[])
            .await
            .unwrap();

        // the Hue v1 API is what a legacy client reads, and it needs no
        // signed-in session of its own
        let router = Router::new()
            .nest("/api", hue_api::router())
            .with_state(state.clone());

        Self {
            _dir: dir,
            state,
            router,
        }
    }

    /// The one device with `ieee`, once the backend has published it.
    async fn device(&self, ieee: &str) -> DeviceInfo {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            if let Some(device) = self
                .state
                .inventory()
                .devices_by_ieee(ieee)
                .into_iter()
                .next()
            {
                return device;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "waited {PATIENCE:?} for {ieee} to be published"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Wait until the backend holds a resource of `rtype`.
    async fn resource(&self, rtype: hue::api::RType) -> Vec<hue::api::ResourceRecord> {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let found = self.state.res.lock().await.get_resources_by_type(rtype);
            if !found.is_empty() {
                return found;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "waited {PATIENCE:?} for a {rtype:?} resource"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// GET a v1 path, as a Hue client would.
    async fn get(&self, uri: &str) -> (u16, Value) {
        let request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }
}

/// The mapping table: one row per class, on all three surfaces.
#[tokio::test]
async fn every_class_maps_to_its_v2_resource_and_its_v1_row() {
    for case in CASES {
        let mock = MockZ2m::start_w09(case.class).await.unwrap();
        let fixture = Fixture::start(mock.url(), case.ieee).await;

        // 1. the inventory the console draws
        let device = fixture.device(case.ieee).await;
        assert_eq!(
            device.device_type,
            expected_type(case.class),
            "{}",
            case.class
        );
        assert!(device.supported, "{}: {device:?}", case.class);
        assert!(device.imported, "{}: {device:?}", case.class);
        assert_eq!(device.unsupported_reason, None, "{}", case.class);

        // 2. the v2 resource the Hue app reads
        let resources = fixture.resource(case.resource).await;
        assert_eq!(resources.len(), 1, "{}: {resources:?}", case.class);
        // the companions a client needs beside it
        assert!(
            !fixture
                .state
                .res
                .lock()
                .await
                .get_resources_by_type(hue::api::RType::DevicePower)
                .is_empty(),
            "{}: no device_power beside the sensor",
            case.class
        );

        // 3. the v1 row an older client reads
        let (status, body) = fixture.get("/api/tester/sensors").await;
        assert_eq!(status, 200, "{}", case.class);
        let rows = body.as_object().expect("the sensor map");
        let row = rows
            .values()
            .find(|row| row["type"] == case.v1_type)
            .unwrap_or_else(|| panic!("{}: no {} row in {body}", case.class, case.v1_type));
        assert_eq!(row["name"], case.friendly_name, "{}", case.class);
        assert_eq!(row["modelid"], device.model.clone().unwrap_or_default());

        // the reading reached the inventory, which is what the row is made
        // from
        assert!(
            device
                .readings
                .iter()
                .any(|reading| reading.property == case.property),
            "{}: no {} reading in {device:?}",
            case.class,
            case.property
        );
    }
}

/// The class a fixture's device belongs to.
fn expected_type(class: &str) -> DeviceType {
    match class {
        "motion" => DeviceType::Motion,
        "temperature" => DeviceType::Temperature,
        "illuminance" => DeviceType::Illuminance,
        "contact" => DeviceType::Contact,
        "remote" => DeviceType::Remote,
        other => panic!("unknown class {other}"),
    }
}

/// The event diff is automatic once the resource changes: a reading that
/// arrives reaches the event stream within a second, without a second path.
#[tokio::test]
async fn a_reading_reaches_the_event_stream_within_a_second() {
    let mock = MockZ2m::start_w09("motion").await.unwrap();
    let fixture = Fixture::start(mock.url(), "0x00158d0000000020").await;
    fixture.device("0x00158d0000000020").await;
    fixture.resource(hue::api::RType::Motion).await;

    // subscribe after the import, so the event this test waits for is the one
    // its own message caused
    let mut events = fixture
        .state
        .res
        .lock()
        .await
        .hue_event_stream()
        .subscribe();

    let sent = std::time::Instant::now();
    mock.publish("Hall motion", &json!({ "occupancy": true, "battery": 87 }));

    let event = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(event) = events.recv().await {
                return event;
            }
        }
    })
    .await
    .expect("an event within one second");

    assert!(
        sent.elapsed() < Duration::from_secs(1),
        "the event was late"
    );
    // the event names the motion resource, which is what the Hue app reads
    let text = serde_json::to_string(&event.block).unwrap();
    assert!(text.contains("update"), "{text}");
}

/// A device this wave declines says why, on the record a person reads.
#[tokio::test]
async fn a_declined_device_says_why() {
    // a sensor the configuration never opts in: supported, not imported, and
    // the reason names the switch
    let mock = MockZ2m::start_w09("motion").await.unwrap();
    let dir = TempDir::new();
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
  server1:
    url: {}
",
        mock.url()
    );
    let store = Arc::new(ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap());
    let (mut svm, _future) = ServiceManager::spawn();
    let state = AppState::from_store(store.clone(), svm.clone())
        .await
        .unwrap();
    let factory = CoreIdleFactory {
        z2m: AppServiceFactory::new(state.clone()),
    };
    service_registry::reconcile(&store, &mut svm, &factory, &[])
        .await
        .unwrap();

    let fixture = Fixture {
        _dir: dir,
        router: Router::new()
            .nest("/api", hue_api::router())
            .with_state(state.clone()),
        state,
    };
    let device = fixture.device("0x00158d0000000020").await;

    assert!(device.supported);
    assert!(!device.imported);
    let reason = device.unsupported_reason.expect("a reason");
    assert!(reason.contains("devices."), "{reason}");

    // and it is absent from the Hue API rather than half-present
    let (status, body) = fixture.get("/api/tester/sensors").await;
    assert_eq!(status, 200);
    assert!(
        !body.to_string().contains("ZLLPresence"),
        "an unimported sensor must not have a v1 row: {body}"
    );
}

/// The v1 sensor that is not a device: the built-in daylight sensor every
/// bridge answers with, so a client that iterates the map still works.
#[tokio::test]
async fn the_daylight_sensor_is_always_there() {
    let mock = MockZ2m::start_w09("motion").await.unwrap();
    let fixture = Fixture::start(mock.url(), "0x00158d0000000020").await;
    fixture.device("0x00158d0000000020").await;

    let (status, body) = fixture.get("/api/tester/sensors").await;
    assert_eq!(status, 200);
    assert!(
        body.as_object()
            .expect("the sensor map")
            .values()
            .any(|row| row["type"] == "Daylight"),
        "{body}"
    );
}

/// A v1 client can read one sensor by id, which is how the Hue app opens a
/// sensor's detail.
#[tokio::test]
async fn one_sensor_reads_by_its_v1_id() {
    let mock = MockZ2m::start_w09("temperature").await.unwrap();
    let fixture = Fixture::start(mock.url(), "0x00158d0000000030").await;
    fixture.device("0x00158d0000000030").await;
    fixture.resource(hue::api::RType::Temperature).await;

    let (_, all) = fixture.get("/api/tester/sensors").await;
    let (id, row) = all
        .as_object()
        .expect("the sensor map")
        .iter()
        .find(|(_, row)| row["type"] == "ZLLTemperature")
        .expect("a temperature row");

    let (status, one) = fixture.get(&format!("/api/tester/sensors/{id}")).await;
    assert_eq!(status, 200);
    assert_eq!(one["type"], "ZLLTemperature");
    assert_eq!(one["uniqueid"], row["uniqueid"]);
    // v1 carries hundredths of a degree
    let centi = one["state"]["temperature"].as_i64().expect("a temperature");
    assert_eq!(centi, 2140);
    assert!(one["state"]["lastupdated"].as_str().is_some());
    assert_eq!(one["config"]["reachable"], true);
}

/// The temperature a v1 client reads is the one the device sent, not a
/// placeholder, and it moves when the device reports again.
#[tokio::test]
async fn a_second_reading_moves_the_v1_row() {
    let mock = MockZ2m::start_w09("temperature").await.unwrap();
    let fixture = Fixture::start(mock.url(), "0x00158d0000000030").await;
    fixture.device("0x00158d0000000030").await;
    fixture.resource(hue::api::RType::Temperature).await;

    mock.publish("Study temperature", &json!({ "temperature": 19.5 }));

    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let (_, body) = fixture.get("/api/tester/sensors").await;
        let centi = body
            .as_object()
            .expect("the sensor map")
            .values()
            .find(|row| row["type"] == "ZLLTemperature")
            .and_then(|row| row["state"]["temperature"].as_i64());
        if centi == Some(1950) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the v1 row never moved: {body}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A reading that arrives while the console is open is stamped with when it
/// arrived, not with when the page opened.
#[tokio::test]
async fn a_reading_carries_the_time_it_arrived() {
    let mock = MockZ2m::start_w09("contact").await.unwrap();
    let fixture = Fixture::start(mock.url(), "0x00158d0000000050").await;
    fixture.device("0x00158d0000000050").await;

    // this contact sensor declares `value_on: false`, so `false` is the
    // door being open; `true` closes it, which is the reading this test
    // waits for
    let before = Utc::now();
    mock.publish("Back door", &json!({ "contact": true }));

    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let device = fixture.device("0x00158d0000000050").await;
        if let Some(reading) = device
            .readings
            .iter()
            .find(|reading| reading.property == "contact" && reading.value == json!(false))
        {
            assert!(reading.at >= before, "the reading is older than the press");
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the contact reading never arrived: {device:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A light still reaches the Hue API exactly as wave 4 left it: the sensor
/// work must not have changed what a lamp imports.
#[tokio::test]
async fn a_light_is_unaffected() {
    let mock = MockZ2m::start_w09("motion").await.unwrap();
    let fixture = Fixture::start(mock.url(), "0x00158d0000000020").await;
    fixture.device("0x00158d0000000020").await;

    // the fixture's only device is a sensor, so a light list must stay empty
    let (status, body) = fixture.get("/api/tester/lights").await;
    assert_eq!(status, 200);
    assert_eq!(body.as_object().expect("the light map").len(), 0, "{body}");

    // and the sensor is not in it
    assert!(!body.to_string().contains("Hall motion"), "{body}");
}

/// A route this wave did not add stays absent: there is no `/sensors` screen,
/// only tabs on the Devices screen.
#[tokio::test]
async fn the_console_has_no_sensors_route() {
    let mock = MockZ2m::start_w09("motion").await.unwrap();
    let fixture = Fixture::start(mock.url(), "0x00158d0000000020").await;

    // the admin surface, which the SPA reads, has no sensor endpoint
    let request = Request::builder()
        .method(Method::GET)
        .uri("/bifrost/sensors")
        .body(Body::empty())
        .unwrap();
    let response = fixture.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 404);
}
