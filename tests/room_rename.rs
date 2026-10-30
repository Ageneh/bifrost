//! A room rename reaches the Hue API after apply, and costs a restart.
//!
//! `rooms` classifies as `RestartBackend`, not hot, so the honest test
//! asserts that the owning backend was re-created and that the room the Hue
//! API answers with carries the new name and icon. The backend, the service
//! registry and the resource store are the real ones.

mod support;

use std::sync::Arc;
use std::time::Duration;

use svc::manager::{ServiceManager, SvmClient};
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service, ServiceState};

use bifrost::config::AppConfig;
use bifrost::config::diff::ApplyAction;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use hue::api::{RType, Room, RoomArchetype};
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

/// How long a test waits for the import and the restart.
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

    /// The room the Hue API would answer with, once it has been imported.
    async fn room(&self, name: &str) -> Option<(String, RoomArchetype)> {
        let res = self.state.res.lock().await;
        res.get_resource_ids_by_type(RType::Room)
            .into_iter()
            .filter_map(|rid| res.get_id::<Room>(rid).ok())
            .find(|room| room.metadata.name == name)
            .map(|room| (room.metadata.name.clone(), room.metadata.archetype))
    }

    async fn wait_for_room(&self, name: &str) -> (String, RoomArchetype) {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            if let Some(room) = self.room(name).await {
                return room;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "waited {PATIENCE:?} for the room {name}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The id the backend is registered under, and its state.
    async fn backend(&mut self) -> (uuid::Uuid, ServiceState) {
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
async fn a_rename_reaches_the_hue_api_and_restarts_the_owning_backend() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let mut fixture = Fixture::start(mock.url()).await;

    fixture.wait_for_room("Kitchen").await;
    let (first_id, _) = fixture.backend().await;
    fixture.svm.wait_for_start(first_id).await.unwrap();

    let mut config = fixture.store.active();
    config.rooms.insert(
        "Kitchen".into(),
        bifrost::config::RoomConfig {
            name: Some("Cooking".into()),
            icon: Some(RoomArchetype::Kitchen),
            ..bifrost::config::RoomConfig::default()
        },
    );

    let revision = fixture.store.revision();
    fixture.store.set_draft(revision, config).unwrap();
    let outcome = fixture.store.apply(revision + 1).unwrap();

    // a rename is not free: the backend that owns the room is re-created
    assert_eq!(
        outcome.plan.hot,
        [ApplyAction::RestartBackend {
            name: "server1".into()
        }]
    );
    assert_eq!(outcome.plan.restart_required, []);

    fixture.reconcile(&outcome.plan.hot).await;
    fixture.svm.wait_for_start("z2m@server1").await.unwrap();

    let (second_id, _) = fixture.backend().await;
    assert_ne!(first_id, second_id, "the backend was not re-created");

    // and the Hue API answers with the new name and icon
    let (name, archetype) = fixture.wait_for_room("Cooking").await;
    assert_eq!(name, "Cooking");
    assert_eq!(archetype, RoomArchetype::Kitchen);
    assert!(
        fixture.room("Kitchen").await.is_none(),
        "the old name is still a room"
    );
}
