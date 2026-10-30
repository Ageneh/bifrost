//! The z2m token leaves the bridge on the websocket, not in a fixture.
//!
//! Wave 2's secret checks read the configuration document and the GET
//! endpoints. The z2m 2.x auth token is not in either: it is appended to the
//! websocket url when the backend connects. This test captures that
//! handshake — the one place the token is visible on the wire — and proves
//! the token goes out there while no fixture on disk carries it.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use svc::manager::{ServiceManager, SvmClient};
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};

use bifrost::config::AppConfig;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

/// A value no fixture may contain; the test greps the tree for it.
const TOKEN: &str = "w10-socket-only-token";

/// Stands in for the core services (http, https, mdns, ssdp, entertainment),
/// which would bind real ports; the z2m backend is the real thing.
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

/// Start Bifrost against `url` with `token` in the z2m server's `token`
/// field, and hand back the mock that saw the handshake.
async fn start_backend(url: &str, token: &str) -> (TempDir, SvmClient) {
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
  office:
    url: {url}
    token: {token}
"
    );
    let store = Arc::new(ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap());
    let (mut svm, _future) = ServiceManager::spawn();
    let state = AppState::from_store(store.clone(), svm.clone())
        .await
        .unwrap();

    let factory = CoreIdleFactory {
        z2m: AppServiceFactory::new(state),
    };
    service_registry::reconcile(&store, &mut svm, &factory, &[])
        .await
        .unwrap();

    (dir, svm)
}

/// Wait until the mock has seen a handshake, or fail after ten seconds.
async fn wait_for_handshake(mock: &MockZ2m) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while mock.connect_urls().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the backend never connected to the mock"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_z2m_token_travels_on_the_websocket_handshake() {
    let mock = MockZ2m::start_w10().await.unwrap();
    let (_dir, _svm) = start_backend(mock.url(), TOKEN).await;

    wait_for_handshake(&mock).await;

    let urls = mock.connect_urls();
    let url = &urls[0];
    assert!(
        url.contains(&format!("token={TOKEN}")),
        "the handshake url carries no token: {url}"
    );
    // z2m 2.x only serves the websocket under /api
    assert!(url.contains("/api?"), "unexpected path: {url}");
}

/// The token is on the wire and nowhere in the tree a fixture could copy it
/// from: the check the wave asks for reads the captured frame, then greps
/// every fixture for the value it saw.
#[tokio::test]
async fn no_fixture_contains_the_token_the_handshake_carried() {
    let mock = MockZ2m::start_w10().await.unwrap();
    let (_dir, _svm) = start_backend(mock.url(), TOKEN).await;
    wait_for_handshake(&mock).await;

    let handshake = mock.connect_urls().remove(0);
    assert!(handshake.contains(TOKEN));

    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = vec![];
    collect_files(&fixtures, &mut files);
    assert!(!files.is_empty(), "no fixtures found under {fixtures:?}");

    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(
            !text.contains(TOKEN),
            "{} contains the token the handshake carried",
            file.display()
        );
    }
}

fn collect_files(directory: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}
