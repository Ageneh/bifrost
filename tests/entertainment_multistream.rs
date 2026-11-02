//! Two DTLS clients, two areas, two zigbee2mqtt servers, one bridge.
//!
//! This is the wave's acceptance: the TV and a second client streaming at the
//! same time, each area on the server that owns its lights, a stop that ends
//! one and leaves the other, and a limit that refuses the third client
//! cleanly.
//!
//! The entertainment service, the backends and the service registry are the
//! real ones. Only the core bridge services are stood in for, so the test
//! binds no privileged port.

mod support;

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use openssl::ssl::{Ssl, SslContext, SslMethod};
use svc::manager::{ServiceManager, SvmClient};
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};
use tokio_openssl::SslStream;
use udp_stream::UdpStream;
use uuid::Uuid;

use bifrost::config::AppConfig;
use bifrost::config::diff::ENTERTAINMENT_SERVICE;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::routes::bifrost::entertainment::status_of;
use bifrost::server::appstate::AppState;
use bifrost::server::entertainment::EntertainmentService;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use bifrost_api::backend::BackendRequest;
use hue::api::{
    Entertainment, EntertainmentConfiguration, EntertainmentConfigurationStatus, Light, LightMode,
    RType,
};
use support::entertainment::{add_area, packet};
use support::mock_z2m::{MockZ2m, fixture_path};
use support::temp_dir::TempDir;

const LIGHT_A: &str = "0x00178801000000a1";
const LIGHT_B: &str = "0x00178801000000b1";

/// The key the bridge answers PSK with; `STANDARD_CLIENT_KEY` in
/// `routes::auth` is the same bytes.
const PSK: &[u8] = b"BifrostHueTlsKey";

const PATIENCE: Duration = Duration::from_secs(10);

/// Stands in for the core services except entertainment, which is the real
/// one on an ephemeral port.
struct CoreFactory {
    z2m: AppServiceFactory,
    state: AppState,
}

impl ServiceFactory for CoreFactory {
    fn build(&self, name: &str, config: &AppConfig) -> ApiResult<BoxDynService> {
        if name == ENTERTAINMENT_SERVICE {
            return Ok(EntertainmentService::new(
                config.bridge.ipaddress,
                config.bridge.entm_port,
                self.state.res.clone(),
                self.state.streams(),
                self.state.store(),
            )?
            .boxed());
        }

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
    /// The UDP port the entertainment service listens on.
    entm_port: u16,
}

impl Fixture {
    /// Start Bifrost against two mock servers, with `max_streams` streams
    /// allowed.
    async fn start(a: &MockZ2m, b: &MockZ2m, max_streams: u32) -> Self {
        let dir = TempDir::new();

        // a free UDP port for the entertainment listener, taken and released
        let probe = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let entm_port = probe.local_addr().unwrap().port();
        drop(probe);

        let yaml = format!(
            "\
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 127.0.0.1
  netmask: 255.0.0.0
  gateway: 127.0.0.1
  timezone: UTC
  entm_port: {entm_port}

z2m:
  serverA:
    url: {}
  serverB:
    url: {}

bifrost:
  entertainment:
    max_streams: {max_streams}
",
            a.url(),
            b.url()
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
            entm_port,
        };
        fixture.reconcile().await;
        fixture
    }

    async fn reconcile(&mut self) {
        let factory = CoreFactory {
            z2m: AppServiceFactory::new(self.state.clone()),
            state: self.state.clone(),
        };
        service_registry::reconcile(&self.store, &mut self.svm, &factory, &[])
            .await
            .unwrap();
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
                "the backends did not import their lights"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn light(&self, ieee: &str) -> hue::api::ResourceLink {
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

    async fn entertainment(&self, light: hue::api::ResourceLink) -> hue::api::ResourceLink {
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

    /// An area over `lights`, already marked streaming the way a v2 `PUT`
    /// would leave it.
    async fn area(&self, name: &str, lights: &[hue::api::ResourceLink]) -> Uuid {
        let mut res = self.state.res.lock().await;
        let area = add_area(&mut res, name, lights);

        for link in lights {
            let ent = res.get::<Entertainment>(link).unwrap();
            let light = ent.renderer_reference.expect("no renderer reference");
            res.update::<Light>(&light.rid, |light| {
                light.mode = LightMode::Streaming;
            })
            .unwrap();
        }

        res.update::<EntertainmentConfiguration>(&area, |ec| {
            ec.status = EntertainmentConfigurationStatus::Active;
        })
        .unwrap();
        drop(res);

        area
    }

    /// The status the diagnostics screen reads.
    fn status(&self) -> Vec<bifrost_api::entertainment::StreamStats> {
        status_of(&self.state).streams
    }
}

/// A DTLS client that sends a frame for `area` every `every`.
struct Client {
    stream: SslStream<UdpStream>,
    area: Uuid,
    sequence: u8,
}

impl Client {
    async fn connect(port: u16, area: Uuid) -> Self {
        let mut ctx = SslContext::builder(SslMethod::dtls_client()).unwrap();
        ctx.set_psk_client_callback(|_ssl, _hint, identity, psk| {
            let id = b"bifrost-test";
            identity[..id.len()].copy_from_slice(id);
            psk[..PSK.len()].copy_from_slice(PSK);
            Ok(PSK.len())
        });
        let ctx = ctx.build();

        let socket = UdpStream::connect(SocketAddr::from(([127, 0, 0, 1], port)))
            .await
            .expect("could not open the UDP socket");
        let ssl = Ssl::new(&ctx).unwrap();
        let mut stream = SslStream::new(ssl, socket).unwrap();

        Pin::new(&mut stream)
            .connect()
            .await
            .expect("the DTLS handshake failed");

        Self {
            stream,
            area,
            sequence: 0,
        }
    }

    /// Send one frame, which is what a Sync client does ten times a second
    /// and more.
    async fn send_frame(&mut self) {
        use tokio::io::AsyncWriteExt;

        let bytes = packet(self.area, self.sequence, &[0]);
        self.sequence = self.sequence.wrapping_add(1);
        self.stream.write_all(&bytes).await.unwrap();
        self.stream.flush().await.unwrap();
    }

    /// Keep streaming for `how_long`, one frame every 20 ms.
    async fn stream_for(&mut self, how_long: Duration) {
        let deadline = tokio::time::Instant::now() + how_long;
        while tokio::time::Instant::now() < deadline {
            self.send_frame().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Two clients on two areas against two servers: two rows in the status,
/// frames rising on both, and each server hearing only its own area.
#[tokio::test]
async fn two_clients_stream_at_once_and_each_server_hears_only_its_own_area() {
    let a = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w07/z2m/server-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b, 2).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let ent_b = fixture.entertainment(fixture.light(LIGHT_B)).await;
    let area_a = fixture.area("TV", &[ent_a]).await;
    let area_b = fixture.area("Desk", &[ent_b]).await;

    let mut client_a = Client::connect(fixture.entm_port, area_a).await;
    let mut client_b = Client::connect(fixture.entm_port, area_b).await;

    client_a.stream_for(Duration::from_millis(400)).await;
    client_b.stream_for(Duration::from_millis(400)).await;

    // both rows exist, with frames on each
    let status = fixture.status();
    assert_eq!(status.len(), 2, "two streams are two rows: {status:?}");

    let row_a = status
        .iter()
        .find(|s| s.area == area_a)
        .expect("no row for A");
    let row_b = status
        .iter()
        .find(|s| s.area == area_b)
        .expect("no row for B");
    assert!(row_a.frames > 0, "no frames counted for A: {row_a:?}");
    assert!(row_b.frames > 0, "no frames counted for B: {row_b:?}");

    // each server heard its own area's frames and nothing else
    let heard_a = a.received_on("TV left/set");
    let heard_b = b.received_on("TV right/set");
    assert!(!heard_a.is_empty(), "the first server heard nothing");
    assert!(!heard_b.is_empty(), "the second server heard nothing");
    assert!(
        b.received_on("TV left/set").is_empty(),
        "the second server heard the first area"
    );
    assert!(
        a.received_on("TV right/set").is_empty(),
        "the first server heard the second area"
    );
}

/// Stopping A leaves B rising, and clears only A's lights and status.
#[tokio::test]
async fn stopping_one_stream_leaves_the_other_rising() {
    let a = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w07/z2m/server-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b, 2).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let ent_b = fixture.entertainment(fixture.light(LIGHT_B)).await;
    let area_a = fixture.area("TV", &[ent_a]).await;
    let area_b = fixture.area("Desk", &[ent_b]).await;

    let mut client_a = Client::connect(fixture.entm_port, area_a).await;
    let mut client_b = Client::connect(fixture.entm_port, area_b).await;
    client_a.stream_for(Duration::from_millis(200)).await;
    client_b.stream_for(Duration::from_millis(200)).await;

    assert_eq!(fixture.status().len(), 2);

    // the admin screen stops A; the session ends on its next frame
    assert!(fixture.state.streams().request_stop(&area_a));
    fixture
        .state
        .res
        .lock()
        .await
        .backend_request(BackendRequest::EntertainmentStop(area_a))
        .unwrap();

    let deadline = tokio::time::Instant::now() + PATIENCE;
    while fixture.status().len() == 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the stopped stream is still listed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // only A's light and status were cleared
    {
        let res = fixture.state.res.lock().await;
        let light_a = res.get_id::<Light>(fixture.light(LIGHT_A).rid).unwrap();
        let light_b = res.get_id::<Light>(fixture.light(LIGHT_B).rid).unwrap();
        assert_eq!(
            light_a.mode,
            LightMode::Normal,
            "A's light is still streaming"
        );
        assert_eq!(light_b.mode, LightMode::Streaming, "B's light was cleared");
        drop(res);
    }

    // and B keeps rising: more frames reach its server after the stop
    let before = b.received_on("TV right/set").len();
    client_b.stream_for(Duration::from_millis(400)).await;

    let deadline = tokio::time::Instant::now() + PATIENCE;
    while b.received_on("TV right/set").len() == before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the second stream stopped with the first"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// With two allowed, the third client is refused and the first two keep
/// streaming.
#[tokio::test]
async fn a_third_client_is_refused_and_the_first_two_keep_streaming() {
    let a = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w07/z2m/server-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b, 2).await;
    fixture.wait_for_lights().await;

    let ent_a = fixture.entertainment(fixture.light(LIGHT_A)).await;
    let ent_b = fixture.entertainment(fixture.light(LIGHT_B)).await;
    let area_a = fixture.area("TV", &[ent_a]).await;
    let area_b = fixture.area("Desk", &[ent_b]).await;

    let mut client_a = Client::connect(fixture.entm_port, area_a).await;
    let mut client_b = Client::connect(fixture.entm_port, area_b).await;
    client_a.stream_for(Duration::from_millis(200)).await;
    client_b.stream_for(Duration::from_millis(200)).await;

    // the third client never completes a handshake: the listener drops its
    // socket instead of starting a session it cannot serve
    let refused = tokio::time::timeout(
        Duration::from_secs(2),
        Client::connect(fixture.entm_port, area_a),
    )
    .await;
    assert!(refused.is_err(), "the third client was accepted");

    // and the two that were there keep going
    let before = a.received_on("TV left/set").len();
    client_a.stream_for(Duration::from_millis(400)).await;
    client_b.stream_for(Duration::from_millis(200)).await;

    let deadline = tokio::time::Instant::now() + PATIENCE;
    while a.received_on("TV left/set").len() == before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the refused client took the first stream with it"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        fixture.status().len(),
        2,
        "the two streams are still listed"
    );
}
