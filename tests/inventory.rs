//! The inventory against a real z2m backend and the mock server.
//!
//! The backend, the service registry and the registry the backend publishes
//! into are the real ones; only the core bridge services are stood in for, so
//! the test binds no privileged port.

mod support;

use std::sync::Arc;
use std::time::Duration;

use svc::manager::{ServiceManager, SvmClient};
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};

use bifrost::config::AppConfig;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::inventory::{DeviceInfo, DeviceType, Inventory};
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

/// How long a test waits for the inventory to catch up with the mock.
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
    /// Start Bifrost against one z2m server, `name`, at `url`.
    async fn start(name: &str, url: &str) -> Self {
        Self::start_with(&[(name, url)]).await
    }

    /// Start Bifrost against several z2m servers.
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

    fn inventory(&self) -> Arc<Inventory> {
        self.state.inventory()
    }

    /// Every device in the inventory right now.
    fn devices(&self) -> Vec<DeviceInfo> {
        self.inventory().snapshot().devices
    }

    /// The one device with `ieee`, once the inventory has it.
    async fn device(&self, ieee: &str) -> DeviceInfo {
        self.wait_until(&format!("device {ieee}"), |devices| {
            devices.iter().any(|device| device.ieee == ieee)
        })
        .await;

        self.inventory()
            .devices_by_ieee(ieee)
            .into_iter()
            .next()
            .expect("the device was just seen")
    }

    /// Wait until `done` holds for the device list.
    ///
    /// # Panics
    ///
    /// Panics when the condition is still false after [`PATIENCE`], naming
    /// what it was waiting for.
    async fn wait_until(&self, what: &str, done: impl Fn(&[DeviceInfo]) -> bool) {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            if done(&self.devices()) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "waited {PATIENCE:?} for {what}; the inventory holds {} device(s)",
                self.devices().len()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[tokio::test]
async fn every_class_is_listed_with_its_type_and_support() {
    // one device per class, each from its own fixture
    for (class, expected, supported) in [
        ("light", DeviceType::Light, true),
        ("plug", DeviceType::Plug, false),
        ("motion", DeviceType::Motion, false),
        ("temperature", DeviceType::Temperature, false),
        ("illuminance", DeviceType::Illuminance, false),
        ("contact", DeviceType::Contact, false),
        ("remote", DeviceType::Remote, false),
    ] {
        let mock = MockZ2m::start_w04(class).await.unwrap();
        let fixture = Fixture::start("server1", mock.url()).await;

        let name = mock.device_names().first().cloned().expect("no device");
        fixture
            .wait_until(&format!("the {class} fixture"), |devices| {
                !devices.is_empty()
            })
            .await;

        let device = fixture
            .devices()
            .into_iter()
            .find(|device| device.friendly_name == name)
            .unwrap_or_else(|| panic!("{name} is missing from the inventory"));

        assert_eq!(device.device_type, expected, "{class}: {device:?}");
        assert_eq!(device.supported, supported, "{class}: {device:?}");
        // every unsupported device says why
        assert_eq!(
            device.unsupported_reason.is_none(),
            supported,
            "{class}: {device:?}"
        );
    }
}

#[tokio::test]
async fn a_light_carries_its_capabilities_and_resources() {
    let mock = MockZ2m::start_w04("light").await.unwrap();
    let fixture = Fixture::start("server1", mock.url()).await;

    let device = fixture.device("0x0017880100000001").await;

    assert_eq!(device.device_type, DeviceType::Light);
    assert!(device.supported);
    assert!(device.capabilities.dimmable);
    assert!(device.capabilities.color_temperature);
    // the Hue resources the device owns, and the room it is in
    assert!(
        device
            .resources
            .iter()
            .any(|resource| resource.rtype == "light"),
        "{device:?}"
    );
    assert_eq!(
        device.room.as_ref().map(|room| room.name.as_str()),
        Some("Kitchen"),
        "{device:?}"
    );
    assert!(device.group.is_some(), "{device:?}");
}

#[tokio::test]
async fn a_removal_drops_the_row_within_two_seconds() {
    let mock = MockZ2m::start_w04("light").await.unwrap();
    let fixture = Fixture::start("server1", mock.url()).await;
    fixture.device("0x0017880100000001").await;

    mock.remove_device("0x0017880100000001");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while fixture
        .inventory()
        .device("server1", "0x0017880100000001")
        .is_some()
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the row was still there after two seconds"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn an_availability_flip_is_visible_in_the_same_second() {
    let mock = MockZ2m::start_w04("light").await.unwrap();
    let fixture = Fixture::start("server1", mock.url()).await;
    fixture.device("0x0017880100000001").await;

    mock.set_availability("Kitchen ceiling", false);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let device = fixture
            .inventory()
            .device("server1", "0x0017880100000001")
            .expect("the device is still listed");
        if device.availability == bifrost::inventory::Availability::Offline {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the flip was not visible within a second: {device:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_server_that_goes_away_leaves_its_devices_unknown_not_online() {
    let mock = MockZ2m::start_w04("light").await.unwrap();
    let fixture = Fixture::start("server1", mock.url()).await;
    fixture.device("0x0017880100000001").await;

    mock.drop_connection();

    fixture
        .wait_until("the server to be marked down", |devices| {
            devices
                .iter()
                .all(|device| device.availability != bifrost::inventory::Availability::Online)
        })
        .await;

    // the devices are still listed: forgetting them is not the same as
    // knowing they are gone
    assert_eq!(fixture.devices().len(), 2);
    let server = fixture.inventory().server("server1").expect("no server");
    assert!(!server.online);
}

#[tokio::test]
async fn two_servers_may_expose_the_same_address() {
    let first = MockZ2m::start_w04("light").await.unwrap();
    let second = MockZ2m::start_w04("light").await.unwrap();
    let fixture = Fixture::start_with(&[("a", first.url()), ("b", second.url())]).await;

    fixture
        .wait_until("both servers' devices", |devices| devices.len() == 4)
        .await;

    let same: Vec<DeviceInfo> = fixture.inventory().devices_by_ieee("0x0017880100000001");
    assert_eq!(same.len(), 2, "the address is one device per server");
    let servers: Vec<&str> = same.iter().map(|device| device.server.as_str()).collect();
    assert_eq!(servers, ["a", "b"]);
}

/// The frames the websocket sends that are an inventory change, in order.
async fn inventory_frames(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    wanted: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    use futures::StreamExt;

    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let frame = tokio::time::timeout_at(deadline, socket.next())
            .await
            .expect("no frame before the deadline")
            .expect("the socket closed")
            .expect("the socket failed");
        let text = frame.into_text().expect("a binary frame");
        let value: serde_json::Value = serde_json::from_str(&text).expect("not JSON");

        if let Some(inventory) = value.get("Inventory")
            && wanted(inventory)
        {
            return inventory.clone();
        }
    }
}

#[tokio::test]
async fn the_websocket_carries_the_inventory_and_every_change() {
    use axum::routing::any;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let mock = MockZ2m::start_w04("light").await.unwrap();
    let fixture = Fixture::start("server1", mock.url()).await;
    fixture.device("0x0017880100000001").await;

    // the websocket handler alone, so the test needs no session: the guard
    // over the whole nest is wave 2's, and already tested there
    let router: axum::Router = axum::Router::new()
        .route("/ws", any(bifrost::routes::bifrost::websocket::websocket))
        .with_state(fixture.state.clone());

    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum_server::from_tcp(listener)
            .serve(router.into_make_service())
            .await;
    });

    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut socket, _) =
        tokio_tungstenite::connect_async(url.as_str().into_client_request().unwrap())
            .await
            .unwrap();

    // the first inventory frame is the whole snapshot
    let snapshot = inventory_frames(&mut socket, |update| update["kind"] == "snapshot").await;
    let devices = snapshot["devices"].as_array().unwrap();
    assert!(
        devices
            .iter()
            .any(|device| device["ieee"] == "0x0017880100000001"),
        "{snapshot}"
    );

    // then a removal arrives as its own change
    mock.remove_device("0x0017880100000001");
    let removed = inventory_frames(&mut socket, |update| {
        update["kind"] == "removed" && update["ieee"] == "0x0017880100000001"
    })
    .await;
    assert_eq!(removed["server"], "server1");
}

#[tokio::test]
async fn permit_join_sends_the_exact_payload_to_one_server() {
    use axum::body::Body;
    use axum::http::{Method, Request, header};
    use tower::ServiceExt;

    let first = MockZ2m::start_w04("light").await.unwrap();
    let second = MockZ2m::start_w04("light").await.unwrap();
    let fixture = Fixture::start_with(&[("a", first.url()), ("b", second.url())]).await;
    fixture
        .wait_until("both servers' devices", |devices| devices.len() == 4)
        .await;

    let router = bifrost::routes::bifrost::z2m::router().with_state(fixture.state.clone());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/a/permit_join")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"time": 60}"#))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), 200, "the permit join was refused");

    // the payload z2m receives, exactly
    let sent = first.wait_for("bridge/request/permit_join").await;
    assert_eq!(sent, serde_json::json!({"time": 60, "device": null}));

    // the other server was never asked to open a window
    assert!(
        second.received_on("bridge/request/permit_join").is_empty(),
        "the permit join was broadcast to every server"
    );
}

#[tokio::test]
async fn identify_reaches_exactly_one_server() {
    let first = MockZ2m::start_w04("light").await.unwrap();
    let second = MockZ2m::start_w04("light").await.unwrap();
    let fixture = Fixture::start_with(&[("a", first.url()), ("b", second.url())]).await;
    fixture
        .wait_until("both servers' devices", |devices| devices.len() == 4)
        .await;

    let device = fixture
        .inventory()
        .device("a", "0x0017880100000001")
        .expect("no device on server a");

    let (reply, answer) = tokio::sync::oneshot::channel();
    fixture
        .state
        .backends()
        .sender("a")
        .expect("server a has no command channel")
        .send(bifrost::backend::z2m::Z2mCommand::Identify {
            device: device.friendly_name.clone(),
            seconds: 3,
            reply,
        })
        .unwrap();
    tokio::time::timeout(PATIENCE, answer)
        .await
        .expect("the backend did not answer")
        .expect("the reply was dropped")
        .expect("the identify was refused");

    let sent = first.wait_for("bridge/request/device/identify").await;
    assert_eq!(sent["name"], "Kitchen ceiling");
    assert_eq!(sent["value"], 3);

    // the other server never heard about it
    assert!(
        second
            .received_on("bridge/request/device/identify")
            .is_empty(),
        "the identify was broadcast"
    );
}
