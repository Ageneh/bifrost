//! Applying a changed z2m server restarts its backend, against the mock.
//!
//! The real backend and the real service registry run here; only the core
//! bridge services are stood in for, so the test binds no privileged port.

mod support;

use std::sync::Arc;

use svc::manager::{ServiceManager, SvmClient};
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service, ServiceState};

use bifrost::config::AppConfig;
use bifrost::config::diff::ApplyAction;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

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
    async fn start(url: &str) -> Self {
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
"
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
        fixture.reconcile(&[]).await;
        fixture
    }

    async fn reconcile(&mut self, actions: &[ApplyAction]) {
        let factory = CoreIdleFactory {
            z2m: AppServiceFactory::new(self.state.clone()),
        };
        service_registry::reconcile(&self.store, &mut self.svm, &factory, actions)
            .await
            .unwrap();
    }

    /// Change the first z2m server's url through the draft, apply, and
    /// reconcile with the plan the apply reported.
    async fn apply_url(&mut self, url: &str) {
        let mut config = self.store.active();
        config.z2m.servers.get_mut("server1").unwrap().url = url.parse().unwrap();

        let revision = self.store.revision();
        self.store.set_draft(revision, config).unwrap();
        let outcome = self.store.apply(revision + 1).unwrap();

        assert_eq!(
            outcome.plan.hot,
            [ApplyAction::RestartBackend {
                name: "server1".into()
            }]
        );
        self.reconcile(&outcome.plan.hot).await;
    }

    async fn backend_id(&mut self) -> (uuid::Uuid, ServiceState) {
        let mut found = None;
        for (id, name) in self.svm.list().await.unwrap() {
            if name.to_string() == "z2m@server1" {
                found = Some(id);
            }
        }
        let id = found.expect("no z2m@server1 backend is registered");
        let state = self.svm.status(id).await.unwrap();
        (id, state)
    }
}

#[tokio::test]
async fn a_changed_url_re_registers_z2m_server1() {
    let mock = MockZ2m::start_w03().await.unwrap();
    let mut fixture = Fixture::start(mock.url()).await;

    let (first_id, _) = fixture.backend_id().await;
    fixture.svm.wait_for_start(first_id).await.unwrap();

    // a different url that names the same mock, so the reconnect succeeds
    fixture.apply_url(&format!("{}/api", mock.url())).await;

    let (second_id, _) = fixture.backend_id().await;
    assert_ne!(first_id, second_id, "the backend was not re-registered");
    fixture.svm.wait_for_start(second_id).await.unwrap();
    assert_eq!(fixture.backend_id().await.1, ServiceState::Running);

    // one backend is registered under the name, not two
    let names: Vec<String> = fixture
        .svm
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|(_, name)| name.to_string())
        .filter(|name| name == "z2m@server1")
        .collect();
    assert_eq!(names, ["z2m@server1"]);
}

#[tokio::test]
async fn the_restarted_backend_connects_to_the_mock_again() {
    let mock = MockZ2m::start_w03().await.unwrap();
    let mut fixture = Fixture::start(mock.url()).await;

    fixture.svm.wait_for_start("z2m@server1").await.unwrap();
    let before = mock.connections();
    assert!(before >= 1, "the first backend never connected");

    fixture.apply_url(&format!("{}/api", mock.url())).await;
    fixture.svm.wait_for_start("z2m@server1").await.unwrap();

    // wait for the second connection, which proves the new instance dialled
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while mock.connections() <= before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the restarted backend did not connect"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
