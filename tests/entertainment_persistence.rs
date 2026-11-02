//! Entertainment configurations survive a zigbee2mqtt reconnect.
//!
//! The editor saves through the Hue v2 `PUT`, which writes the configuration
//! into the resources and, through them, into `state.yaml`. Nothing about a
//! zone lives in `config.yaml`. A reconnect re-imports every device from
//! zigbee2mqtt, and that import must leave the zones an editor saved alone.

mod support;

use std::sync::Arc;
use std::time::Duration;

use svc::manager::{ServiceManager, SvmClient};
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};

use bifrost::config::AppConfig;
use bifrost::config::diff::ApplyAction;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::routes::clip::entertainment_configuration::put_resource_id;
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use hue::api::{EntertainmentConfiguration, RType, ResourceLink};
use serde_json::json;
use support::mock_z2m::{MockZ2m, fixture_path};
use support::temp_dir::TempDir;

const LIGHT_A: &str = "0x00178801000000a1";

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
    async fn start(mock: &MockZ2m) -> Self {
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

    /// Restart the z2m backend, which dials the server again and re-imports
    /// every device: what a reconnect looks like to the resources.
    async fn restart_backend(&mut self) {
        let factory = CoreIdleFactory {
            z2m: AppServiceFactory::new(self.state.clone()),
        };
        service_registry::reconcile(
            &self.store,
            &mut self.svm,
            &factory,
            &[ApplyAction::RestartBackend {
                name: "server1".into(),
            }],
        )
        .await
        .unwrap();
    }

    /// Wait until the light `ieee` is imported.
    async fn wait_for_light(&self) {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let ready = self
                .state
                .inventory()
                .devices_by_ieee(LIGHT_A)
                .first()
                .is_some_and(|device| device.resources.iter().any(|r| r.rtype == "light"));
            if ready {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the light was not imported"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The light service the import created for `ieee`.
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

    /// The entertainment service that renders `light`.
    async fn entertainment(&self, light: ResourceLink) -> ResourceLink {
        let res = self.state.res.lock().await;
        res.get_resource_ids_by_type(RType::Entertainment)
            .into_iter()
            .find(|id| {
                res.get_id::<hue::api::Entertainment>(*id)
                    .is_ok_and(|ent| ent.renderer_reference == Some(light))
            })
            .map(|id| RType::Entertainment.link_to(id))
            .expect("no entertainment service")
    }
}

/// An editor's positions come back unchanged after the server reconnects.
#[tokio::test]
async fn a_reconnect_keeps_the_positions_an_editor_saved() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_light().await;

    let light = fixture.light(LIGHT_A);
    let service = fixture.entertainment(light).await;

    // create the zone the way the Hue app does, then move its light where
    // the editor put it
    let area = {
        let res = fixture.state.res.lock().await;
        let new = json!({
            "configuration_type": "screen",
            "metadata": {"name": "Living room"},
            "stream_proxy": null,
            "locations": {
                "service_locations": [
                    {"positions": [{"x": -0.4, "y": 0.8, "z": -0.4}], "service": {"rid": service.rid, "rtype": "entertainment"}},
                ],
            },
        });
        drop(res);

        let reply =
            bifrost::routes::clip::entertainment_configuration::post_resource(&fixture.state, new)
                .await
                .expect("the zone was not created");
        let _ = reply;
        let res = fixture.state.res.lock().await;
        res.get_resource_ids_by_type(RType::EntertainmentConfiguration)
            .first()
            .copied()
            .expect("no zone")
    };

    let link = ResourceLink::new(area, RType::EntertainmentConfiguration);
    put_resource_id(
        &fixture.state,
        link,
        json!({
            "locations": {
                "service_locations": [
                    {"positions": [{"x": 0.25, "y": 0.5, "z": 0.75}], "service": {"rid": service.rid, "rtype": "entertainment"}},
                ],
            },
        }),
    )
    .await
    .expect("the positions were not saved");

    let saved = {
        let res = fixture.state.res.lock().await;
        let ec = res.get_id::<EntertainmentConfiguration>(area).unwrap();
        let positions = ec.locations.service_locations[0].positions.clone();
        drop(res);
        positions
    };
    assert_eq!(saved[0].x, 0.25, "the editor's position was not stored");

    // the state file the running bridge writes, as the next start reads it
    let persisted = fixture.state.res.lock().await.serialize().unwrap();
    assert!(
        persisted.contains("0.25"),
        "the zone is not in the serialized state"
    );

    // the backend reconnects and re-imports every device
    let mut fixture = fixture;
    let before = mock.connections();
    fixture.restart_backend().await;

    let deadline = tokio::time::Instant::now() + PATIENCE;
    while mock.connections() <= before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the backend did not reconnect"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    fixture.wait_for_light().await;

    let after = {
        let res = fixture.state.res.lock().await;
        let ec = res.get_id::<EntertainmentConfiguration>(area).unwrap();
        let positions = ec.locations.service_locations[0].positions.clone();
        drop(res);
        positions
    };

    assert_eq!(
        after[0].x, 0.25,
        "the reconnect overwrote the position the editor saved"
    );
    assert_eq!(after[0].y, 0.5);
    assert_eq!(after[0].z, 0.75);
}

/// A zone with no lights yet takes a start without failing: the editor
/// creates one before it has chosen any light.
#[tokio::test]
async fn a_zone_with_no_lights_takes_an_action() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_light().await;

    let bridge = {
        let res = fixture.state.res.lock().await;
        let bridge = res
            .get_resource_ids_by_type(RType::Bridge)
            .first()
            .copied()
            .expect("no bridge");
        drop(res);
        bridge
    };

    let node = RType::Entertainment.deterministic(bridge.to_string());
    let link = ResourceLink::new(uuid::Uuid::new_v4(), RType::EntertainmentConfiguration);

    {
        let mut res = fixture.state.res.lock().await;
        res.add(
            &link,
            hue::api::Resource::EntertainmentConfiguration(EntertainmentConfiguration {
                name: "Empty".into(),
                configuration_type: hue::api::EntertainmentConfigurationType::Screen,
                metadata: hue::api::EntertainmentConfigurationMetadata {
                    name: "Empty".into(),
                },
                status: hue::api::EntertainmentConfigurationStatus::Inactive,
                stream_proxy: hue::api::EntertainmentConfigurationStreamProxy {
                    mode: hue::api::EntertainmentConfigurationStreamProxyMode::Auto,
                    node,
                },
                locations: hue::api::EntertainmentConfigurationLocations {
                    service_locations: vec![],
                },
                light_services: vec![],
                channels: vec![],
                active_streamer: None,
            }),
        )
        .unwrap();
        drop(res);
    }

    put_resource_id(&fixture.state, link, json!({"action": "start"}))
        .await
        .expect("an empty zone could not be started");

    let area = link.rid;

    let streaming = {
        let res = fixture.state.res.lock().await;
        let ec = res.get_id::<EntertainmentConfiguration>(area).unwrap();
        let streaming = ec.is_streaming();
        drop(res);
        streaming
    };
    assert!(streaming);
}

/// A zone is a Hue resource, not a configuration entry: the settings file
/// never learns about it.
#[tokio::test]
async fn a_zone_is_not_written_into_the_configuration() {
    let mock = MockZ2m::start(&fixture_path("w07/z2m/server-a.json"))
        .await
        .unwrap();
    let fixture = Fixture::start(&mock).await;
    fixture.wait_for_light().await;

    let light = fixture.light(LIGHT_A);
    let service = fixture.entertainment(light).await;

    bifrost::routes::clip::entertainment_configuration::post_resource(
        &fixture.state,
        json!({
            "configuration_type": "screen",
            "metadata": {"name": "Living room"},
            "stream_proxy": null,
            "locations": {
                "service_locations": [
                    {"positions": [{"x": 0.0, "y": 0.0, "z": 0.0}], "service": {"rid": service.rid, "rtype": "entertainment"}},
                ],
            },
        }),
    )
    .await
    .expect("the zone was not created");

    let written = std::fs::read_to_string(&fixture.store.paths().config).unwrap();
    assert!(
        !written.contains("Living room"),
        "the zone leaked into the configuration: {written}"
    );

    // and it is in the state, which is where a zone belongs
    let state = fixture.state.res.lock().await.serialize().unwrap();
    assert!(
        state.contains("Living room"),
        "the zone is not in the state"
    );
}
