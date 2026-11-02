//! The soak: four streams for thirty minutes, with inventory load on top.
//!
//! Ticket 195 asks for four streams, no panic, RSS growth under 10 %, and a
//! drop rate that does not climb; the `Resources` mutex is measured under
//! the same load. The test is `#[ignore]`d because it takes half an hour;
//! `scripts/soak.sh` runs it and records the numbers.
//!
//! Everything here is the real bridge except the two z2m servers, which are
//! the mock, and the core listeners, which would bind ports: the
//! entertainment service, the backends, the inventory and the resource
//! registry are the production ones.

mod support;

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
use hue::api::{
    Entertainment, EntertainmentConfiguration, EntertainmentConfigurationStatus, Light, LightMode,
    RType,
};
use support::entertainment::{add_area, packet};
use support::mock_z2m::{MockZ2m, fixture_path};
use support::temp_dir::TempDir;

const LIGHTS: [&str; 4] = [
    "0x00178801000000a1",
    "0x00178801000000a2",
    "0x00178801000000b1",
    "0x00178801000000b2",
];

/// The key the bridge answers PSK with; `STANDARD_CLIENT_KEY` in
/// `routes::auth` is the same bytes.
const PSK: &[u8] = b"BifrostHueTlsKey";

const PATIENCE: Duration = Duration::from_secs(10);

/// How long the soak runs. `BIFROST_SOAK_MINUTES` shortens it for a smoke
/// run; the gate uses the default.
fn soak_duration() -> Duration {
    let minutes = std::env::var("BIFROST_SOAK_MINUTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    Duration::from_secs(minutes * 60)
}

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
    entm_port: u16,
}

impl Fixture {
    async fn start(a: &MockZ2m, b: &MockZ2m) -> Self {
        let dir = TempDir::new();

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
    max_streams: 4
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
            let ready = LIGHTS.iter().all(|ieee| {
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

        area
    }

    fn status(&self) -> Vec<bifrost_api::entertainment::StreamStats> {
        status_of(&self.state).streams
    }

    /// How long acquiring the `Resources` mutex takes right now.
    async fn lock_latency(&self) -> Duration {
        let started = Instant::now();
        let _guard = self.state.res.lock().await;
        started.elapsed()
    }
}

/// A DTLS client that sends a frame every 20 ms.
struct Client {
    stream: SslStream<UdpStream>,
    area: Uuid,
    sequence: u8,
}

impl Client {
    async fn connect(port: u16, area: Uuid) -> Self {
        let mut ctx = SslContext::builder(SslMethod::dtls_client()).unwrap();
        ctx.set_psk_client_callback(|_ssl, _hint, identity, psk| {
            let id = b"bifrost-soak";
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

    async fn send_frame(&mut self) {
        use tokio::io::AsyncWriteExt;

        let bytes = packet(self.area, self.sequence, &[0]);
        self.sequence = self.sequence.wrapping_add(1);
        self.stream.write_all(&bytes).await.unwrap();
        self.stream.flush().await.unwrap();
    }

    async fn stream_for(mut self, how_long: Duration) {
        let deadline = tokio::time::Instant::now() + how_long;
        while tokio::time::Instant::now() < deadline {
            self.send_frame().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Resident set size of this process, in KiB.
fn rss_kib() -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps did not run");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("ps printed no rss")
}

/// Four streams for the configured duration, with an inventory report every
/// 30 seconds, sampling RSS, mutex latency and the drop ratio.
///
/// The assertions are the wave's numbers: RSS growth under 10 %, no panic
/// (the test reaching the end), and a drop ratio that does not climb.
#[tokio::test]
#[ignore = "half an hour; scripts/soak.sh runs it"]
#[allow(clippy::cast_precision_loss)] // counters are far below 2^52
async fn four_streams_for_half_an_hour() {
    let a = MockZ2m::start(&fixture_path("w10/soak-a.json"))
        .await
        .unwrap();
    let b = MockZ2m::start(&fixture_path("w10/soak-b.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&a, &b).await;
    fixture.wait_for_lights().await;

    let mut areas = vec![];
    for (index, ieee) in LIGHTS.iter().enumerate() {
        let ent = fixture.entertainment(fixture.light(ieee)).await;
        areas.push(fixture.area(&format!("Soak {index}"), &[ent]).await);
    }

    let run_for = soak_duration();
    let mut clients = vec![];
    for area in &areas {
        let mut client = Client::connect(fixture.entm_port, *area).await;
        // the first frame is what names the area; send it before the run so
        // a refused area is visible here, not half an hour later
        client.send_frame().await;
        clients.push(client);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("rows after the first frames: {}", fixture.status().len());

    // the mock's message history is not what this measures; keeping it
    // would grow this process's heap with every frame and hide the bridge's
    a.stop_recording();
    b.stop_recording();

    // The streams and the sampler run concurrently in one task: `join!`
    // polls both, so neither starves the other and the mutex sample sees
    // the same runtime the streams do. The baseline is taken after a warmup
    // *while streaming*: a session that receives no frame for ten seconds
    // times out, so an idle warmup would end every stream before it began.
    let streaming =
        futures::future::join_all(clients.into_iter().map(|client| client.stream_for(run_for)));

    let sample_every = Duration::from_secs(30);
    let sampling = async {
        let started = Instant::now();
        let warmup = Duration::from_secs(60).min(run_for / 6);
        tokio::time::sleep(warmup).await;
        let baseline = rss_kib();
        let mut samples = vec![];
        // the rows while the streams run: once the clients finish, the
        // registry is empty by design, so the assertions read the last
        // live sample
        let mut last_rows = vec![];
        let mut worst_lock = Duration::ZERO;

        // the last sample lands before the clients stop, so a session that
        // ends on time is not reported as a timeout
        while started.elapsed() + sample_every <= run_for {
            tokio::time::sleep(sample_every).await;

            let latency = fixture.lock_latency().await;
            worst_lock = worst_lock.max(latency);

            // inventory load: every device reports, twice a minute
            for name in ["TV left", "Soak left"] {
                a.publish(name, &serde_json::json!({"state": "ON", "brightness": 128}));
            }
            for name in ["TV right", "Soak right"] {
                b.publish(name, &serde_json::json!({"state": "ON", "brightness": 128}));
            }

            let status = fixture.status();
            let frames: u64 = status.iter().map(|row| row.frames).sum();
            let drops: u64 = status.iter().map(|row| row.drops).sum();
            last_rows = status.clone();
            samples.push((
                started.elapsed().as_secs(),
                rss_kib(),
                latency,
                frames,
                drops,
            ));
            println!(
                "t={:>4}s rss={} KiB lock={latency:?} rows={} frames={frames} drops={drops} errors={:?}",
                samples.last().unwrap().0,
                samples.last().unwrap().1,
                status.len(),
                status
                    .iter()
                    .filter_map(|row| row.last_error.as_deref())
                    .collect::<Vec<_>>()
            );
        }

        (baseline, samples, last_rows, worst_lock)
    };

    let (_done, (baseline, samples, last_rows, worst_lock)) = tokio::join!(streaming, sampling);

    let last = samples.last().expect("no samples").1;
    let growth = (last as f64 - baseline as f64) / baseline as f64 * 100.0;
    let first_ratio = samples
        .first()
        .map_or(0.0, |sample| sample.4 as f64 / sample.3.max(1) as f64);
    let last_ratio = samples
        .last()
        .map_or(0.0, |sample| sample.4 as f64 / sample.3.max(1) as f64);

    println!(
        "soak: baseline={baseline} KiB last={last} KiB growth={growth:.1}% \
         worst_lock={worst_lock:?} first_drop_ratio={first_ratio:.3} last_drop_ratio={last_ratio:.3}"
    );

    assert_eq!(
        last_rows.len(),
        4,
        "four streams are four rows: {last_rows:?}"
    );
    assert!(
        last_rows.iter().all(|row| row.frames > 0),
        "a stream received no frame: {last_rows:?}"
    );
    assert!(growth < 10.0, "RSS grew {growth:.1}%, over the 10% budget");
    assert!(
        last_ratio <= first_ratio + 0.05,
        "the drop ratio climbed from {first_ratio:.3} to {last_ratio:.3}"
    );
    assert!(
        worst_lock < Duration::from_millis(250),
        "the Resources mutex was held for {worst_lock:?}"
    );
}
