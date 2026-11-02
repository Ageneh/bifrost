//! Device overrides at import, against the mock zigbee2mqtt.
//!
//! `devices.<ieee>` decides which devices reach the Hue API at all, what
//! name they show, and which room a hand-written configuration puts them in.
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
use hue::api::{Light, RType, Room};
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

/// How long a test waits for the import to reach the mock's state.
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
    /// Start Bifrost against one z2m server, with `overrides` under
    /// `devices:` exactly as the file spells them.
    async fn start(url: &str, overrides: &str) -> Self {
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
{overrides}"
        );
        let store = Arc::new(ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap());
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(store.clone(), svm.clone())
            .await
            .unwrap();

        let mut svm = svm;
        let factory = CoreIdleFactory {
            z2m: AppServiceFactory::new(state.clone()),
        };
        service_registry::reconcile(&store, &mut svm, &factory, &[])
            .await
            .unwrap();

        Self {
            _dir: dir,
            store,
            svm,
            state,
        }
    }

    /// The name of every Light in the resource store.
    async fn light_names(&self) -> Vec<String> {
        let res = self.state.res.lock().await;
        res.get_resource_ids_by_type(RType::Light)
            .into_iter()
            .filter_map(|rid| res.get_id::<Light>(rid).ok())
            .map(|light| light.metadata.name.clone())
            .collect()
    }

    /// The child links of the room named `name`, if it exists.
    async fn room_children(&self, name: &str) -> Option<Vec<String>> {
        let res = self.state.res.lock().await;
        res.get_resource_ids_by_type(RType::Room)
            .into_iter()
            .filter_map(|rid| res.get_id::<Room>(rid).ok())
            .find(|room| room.metadata.name == name)
            .map(|room| {
                room.children
                    .iter()
                    .map(|link| link.rid.to_string())
                    .collect()
            })
    }

    /// Wait until `done` holds of the imported light names.
    async fn wait_for_lights(&self, done: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let names = self.light_names().await;
            if done(&names) {
                return names;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "waited {PATIENCE:?} for the import; the store holds {names:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[tokio::test]
async fn a_disabled_device_is_not_imported_but_stays_on_the_network() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let fixture = Fixture::start(mock.url(), "  0x0017880100000001:\n    enabled: false\n").await;

    let names = fixture.wait_for_lights(|names| names.len() == 2).await;

    assert!(
        !names.contains(&"Kitchen ceiling".to_string()),
        "the disabled device was imported: {names:?}"
    );
    assert!(names.contains(&"Hall lamp".to_string()), "{names:?}");
    assert!(names.contains(&"Attic bulb".to_string()), "{names:?}");

    // the network still knows it, so the Devices screen can show it
    let inventory = fixture.state.inventory();
    let seen = inventory.devices_by_ieee("0x0017880100000001");
    assert_eq!(seen.len(), 1, "the disabled device left the inventory");
}

#[tokio::test]
async fn a_name_override_is_what_the_hue_api_shows() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let fixture = Fixture::start(
        mock.url(),
        "  0x0017880100000002:\n    name: Hall renamed\n",
    )
    .await;

    let names = fixture
        .wait_for_lights(|names| names.contains(&"Hall renamed".to_string()))
        .await;

    assert!(!names.contains(&"Hall lamp".to_string()), "{names:?}");
    // the override is per device, so the others keep their z2m names
    assert!(names.contains(&"Kitchen ceiling".to_string()), "{names:?}");
}

#[tokio::test]
async fn a_room_override_places_a_device_the_groups_do_not() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let fixture = Fixture::start(mock.url(), "  0x0017880100000003:\n    room: Kitchen\n").await;

    // the group holds two devices; the override adds the third
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        if let Some(children) = fixture.room_children("Kitchen").await {
            if children.len() == 3 {
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the room override was never applied"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let children = fixture.room_children("Kitchen").await.unwrap();

    // the device link the inventory reports for the attic bulb
    let attic = fixture
        .state
        .inventory()
        .devices_by_ieee("0x0017880100000003")
        .into_iter()
        .next()
        .expect("the attic bulb was never seen");
    let device = attic
        .resources
        .iter()
        .find(|resource| resource.rtype == "device")
        .expect("no device resource for the attic bulb");

    assert!(
        children.contains(&device.rid),
        "the overridden device is not a child of the room: {children:?}"
    );
}

#[tokio::test]
async fn an_override_added_by_apply_survives_the_restart_it_causes() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let mut fixture = Fixture::start(mock.url(), "  {}\n").await;

    fixture.wait_for_lights(|names| names.len() == 3).await;

    // `devices` classifies as RestartBackend, so this is not a hot change
    let mut config = fixture.store.active();
    config.devices.insert(
        "0x0017880100000002".into(),
        bifrost::config::DeviceConfig {
            name: Some("Hall renamed".into()),
            ..bifrost::config::DeviceConfig::default()
        },
    );

    let revision = fixture.store.revision();
    fixture.store.set_draft(revision, config).unwrap();
    let outcome = fixture.store.apply(revision + 1).unwrap();
    assert_eq!(
        outcome.plan.hot,
        [bifrost::config::diff::ApplyAction::RestartBackend {
            name: "server1".into()
        }]
    );

    let factory = CoreIdleFactory {
        z2m: AppServiceFactory::new(fixture.state.clone()),
    };
    service_registry::reconcile(
        &fixture.store,
        &mut fixture.svm,
        &factory,
        &outcome.plan.hot,
    )
    .await
    .unwrap();
    fixture.svm.wait_for_start("z2m@server1").await.unwrap();

    let names = fixture
        .wait_for_lights(|names| names.contains(&"Hall renamed".to_string()))
        .await;
    assert!(!names.contains(&"Hall lamp".to_string()), "{names:?}");
}
