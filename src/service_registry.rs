//! Construction and reconciliation of the configuration-driven services.
//!
//! Startup and apply both go through [`reconcile`], so a changed
//! configuration can stop, unregister and re-create services without
//! duplicating how they are built.

use std::collections::BTreeSet;

use svc::manager::SvmClient;
use svc::serviceid::{ServiceId, ServiceName};
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};

use crate::backend::z2m::Z2mServiceTemplate;
use crate::config::AppConfig;
use crate::config::diff::{
    ApplyAction, ENTERTAINMENT_SERVICE, HTTP_SERVICE, HTTPS_SERVICE, MDNS_SERVICE, SSDP_SERVICE,
};
use crate::config_store::ConfigStore;
use crate::error::ApiResult;
use crate::server::appstate::AppState;
use crate::server::entertainment::EntertainmentService;
use crate::server::http::HttpServer;
use crate::server::mdns::MdnsService;
use crate::server::ssdp::SsdpService;
use crate::server::{self, Protocol};

/// Services built from the bridge settings, in registration order.
pub const CORE_SERVICES: &[&str] = &[
    MDNS_SERVICE,
    HTTP_SERVICE,
    HTTPS_SERVICE,
    SSDP_SERVICE,
    ENTERTAINMENT_SERVICE,
];

/// Template generating one backend per configured z2m server.
pub const Z2M_TEMPLATE: &str = "z2m";

/// Builds the services [`reconcile`] manages.
pub trait ServiceFactory: Send + Sync {
    /// Build core service `name` (one of [`CORE_SERVICES`]) for `config`.
    fn build(&self, name: &str, config: &AppConfig) -> ApiResult<BoxDynService>;

    /// Build the template generating z2m backends.
    fn z2m_template(&self) -> Box<dyn ServiceTemplate>;
}

/// The production factory, building services around the shared [`AppState`].
pub struct AppServiceFactory {
    state: AppState,
}

impl AppServiceFactory {
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl ServiceFactory for AppServiceFactory {
    fn build(&self, name: &str, config: &AppConfig) -> ApiResult<BoxDynService> {
        let bridge = &config.bridge;
        let state = &self.state;

        let svc = match name {
            MDNS_SERVICE => MdnsService::new(bridge.mac, bridge.ipaddress).boxed(),
            HTTP_SERVICE => HttpServer::http(
                bridge.ipaddress,
                bridge.http_port,
                server::build_service(Protocol::Http, state.clone()),
            )
            .boxed(),
            HTTPS_SERVICE => HttpServer::https_openssl(
                bridge.ipaddress,
                bridge.https_port,
                server::build_service(Protocol::Https, state.clone()),
                &state.store().paths().cert,
            )?
            .boxed(),
            SSDP_SERVICE => SsdpService::new(bridge.mac, bridge.ipaddress, state.updater()).boxed(),
            ENTERTAINMENT_SERVICE => EntertainmentService::new(
                bridge.ipaddress,
                bridge.entm_port,
                state.res.clone(),
                state.streams(),
                state.store(),
            )?
            .boxed(),
            _ => {
                return Err(svc::error::SvcError::ServiceNotFound(ServiceId::from(
                    name.to_string(),
                ))
                .into());
            }
        };

        Ok(svc)
    }

    fn z2m_template(&self) -> Box<dyn ServiceTemplate> {
        Box::new(Z2mServiceTemplate::new(self.state.clone()))
    }
}

fn z2m_instance(name: &str) -> ServiceId {
    ServiceId::instance(Z2M_TEMPLATE, name)
}

/// Templated instances are registered by their first start and run by the
/// second.
async fn start_backend(svm: &mut SvmClient, name: &str) -> ApiResult<()> {
    let id = svm.start(z2m_instance(name)).await?;
    svm.start(id).await?;
    Ok(())
}

/// Bring the registered services in line with the active configuration.
///
/// Registers and starts missing core services, registers the z2m template,
/// starts backends for added z2m servers and unregisters backends for
/// removed ones. `actions` (the hot part of an apply plan) names the
/// services whose settings changed; those are re-created from the new
/// configuration.
pub async fn reconcile(
    store: &ConfigStore,
    svm: &mut SvmClient,
    factory: &dyn ServiceFactory,
    actions: &[ApplyAction],
) -> ApiResult<()> {
    let config = store.active();

    let registered: BTreeSet<ServiceName> = svm
        .list()
        .await?
        .into_iter()
        .map(|(_, name)| name)
        .collect();

    let mut fresh: BTreeSet<String> = BTreeSet::new();

    for name in CORE_SERVICES {
        if !registered.contains(&ServiceName::from(*name)) {
            svm.register_service(name, factory.build(name, &config)?)
                .await?;
            svm.start(*name).await?;
            fresh.insert((*name).to_string());
        }
    }

    // replacing the template is harmless, and keeps this idempotent
    svm.register_template(Z2M_TEMPLATE, factory.z2m_template())
        .await?;

    let running_backends: BTreeSet<&str> = registered
        .iter()
        .filter(|name| name.name() == Z2M_TEMPLATE)
        .filter_map(ServiceName::instance)
        .collect();

    for name in &running_backends {
        if !config.z2m.servers.contains_key(*name) {
            log::info!("Stopping z2m backend {name:?}, it is no longer configured");
            svm.unregister(z2m_instance(name)).await?;
        }
    }

    for name in config.z2m.servers.keys() {
        if !running_backends.contains(name.as_str()) {
            log::info!("Starting z2m backend {name:?}");
            start_backend(svm, name).await?;
            fresh.insert(format!("{Z2M_TEMPLATE}@{name}"));
        }
    }

    for action in actions {
        match action {
            ApplyAction::RestartService { service } if !fresh.contains(service) => {
                log::info!("Restarting {service} for the new configuration");
                svm.restart_with(service.as_str(), factory.build(service, &config)?)
                    .await?;
            }
            ApplyAction::RestartBackend { name }
                if config.z2m.servers.contains_key(name)
                    && !fresh.contains(&format!("{Z2M_TEMPLATE}@{name}")) =>
            {
                log::info!("Restarting z2m backend {name:?} for the new configuration");
                svm.restart(z2m_instance(name)).await?;
            }
            // updates are read on use; starts and stops were handled above
            _ => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use svc::manager::{ServiceManager, SvmClient};
    use svc::template::ServiceTemplate;
    use svc::traits::{BoxDynService, Service, ServiceState};

    use crate::config::AppConfig;
    use crate::config::diff::{self, ApplyAction, HTTP_SERVICE};
    use crate::config_store::ConfigStore;
    use crate::error::ApiResult;
    use crate::service_registry::{CORE_SERVICES, ServiceFactory, reconcile};
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    struct Idle;

    #[async_trait::async_trait]
    impl Service for Idle {
        type Error = Infallible;

        async fn run(&mut self) -> Result<(), Infallible> {
            futures::future::pending().await
        }
    }

    /// Counts how often each service was built.
    #[derive(Clone, Default)]
    struct CountingFactory {
        builds: Arc<Mutex<BTreeMap<String, usize>>>,
    }

    impl CountingFactory {
        fn count(&self, name: &str) {
            *self
                .builds
                .lock()
                .unwrap()
                .entry(name.to_string())
                .or_default() += 1;
        }

        fn builds(&self, name: &str) -> usize {
            self.builds.lock().unwrap().get(name).copied().unwrap_or(0)
        }
    }

    impl ServiceFactory for CountingFactory {
        fn build(&self, name: &str, _config: &AppConfig) -> ApiResult<BoxDynService> {
            self.count(name);
            Ok(Idle.boxed())
        }

        fn z2m_template(&self) -> Box<dyn ServiceTemplate> {
            let factory = self.clone();
            Box::new(move |instance: String| {
                factory.count(&format!("z2m@{instance}"));
                Ok(Idle.boxed())
            })
        }
    }

    struct Fixture {
        _dir: TempDir,
        store: ConfigStore,
        svm: SvmClient,
        factory: CountingFactory,
    }

    impl Fixture {
        async fn start() -> Self {
            let dir = TempDir::new();
            let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
            let (svm, _future) = ServiceManager::spawn();
            let mut fixture = Self {
                _dir: dir,
                store,
                svm,
                factory: CountingFactory::default(),
            };
            fixture.reconcile(&[]).await;
            fixture
        }

        async fn reconcile(&mut self, actions: &[ApplyAction]) {
            reconcile(&self.store, &mut self.svm, &self.factory, actions)
                .await
                .unwrap();
        }

        /// Apply `change` through the store and reconcile with its plan.
        async fn apply(&mut self, change: impl FnOnce(&mut AppConfig)) {
            let mut config = self.store.active();
            change(&mut config);
            let revision = self.store.revision();
            self.store.set_draft(revision, config).unwrap();
            let outcome = self.store.apply(revision + 1).unwrap();
            assert_eq!(
                outcome.plan,
                diff::plan(&outcome.previous, &outcome.applied)
            );
            self.reconcile(&outcome.plan.hot).await;
        }

        async fn names(&mut self) -> Vec<String> {
            let mut names: Vec<String> = self
                .svm
                .list()
                .await
                .unwrap()
                .into_iter()
                .map(|(_, name)| name.to_string())
                .collect();
            names.sort();
            names
        }

        async fn assert_running(&mut self, name: &str) {
            self.svm.wait_for_start(name.to_string()).await.unwrap();
            assert_eq!(
                self.svm.status(name.to_string()).await.unwrap(),
                ServiceState::Running
            );
        }
    }

    #[tokio::test]
    async fn startup_registers_and_starts_everything() {
        let mut fixture = Fixture::start().await;

        let mut expected: Vec<String> = CORE_SERVICES.iter().map(ToString::to_string).collect();
        expected.push("z2m@server1".into());
        expected.sort();
        assert_eq!(fixture.names().await, expected);

        for name in expected {
            fixture.assert_running(&name).await;
            assert_eq!(fixture.factory.builds(&name), 1, "{name}");
        }
    }

    #[tokio::test]
    async fn reconcile_without_changes_is_idempotent() {
        let mut fixture = Fixture::start().await;
        let before = fixture.names().await;

        fixture.reconcile(&[]).await;

        assert_eq!(fixture.names().await, before);
        assert_eq!(fixture.factory.builds(HTTP_SERVICE), 1);
        assert_eq!(fixture.factory.builds("z2m@server1"), 1);
    }

    #[tokio::test]
    async fn port_change_restarts_only_that_listener() {
        let mut fixture = Fixture::start().await;

        fixture.apply(|config| config.bridge.http_port = 8080).await;

        fixture.assert_running(HTTP_SERVICE).await;
        assert_eq!(fixture.factory.builds(HTTP_SERVICE), 2);
        for name in CORE_SERVICES.iter().filter(|name| **name != HTTP_SERVICE) {
            assert_eq!(fixture.factory.builds(name), 1, "{name}");
        }
    }

    #[tokio::test]
    async fn address_change_restarts_all_network_services() {
        let mut fixture = Fixture::start().await;

        fixture
            .apply(|config| config.bridge.ipaddress = "10.12.0.21".parse().unwrap())
            .await;

        for name in CORE_SERVICES {
            fixture.assert_running(name).await;
            assert_eq!(fixture.factory.builds(name), 2, "{name}");
        }
        assert_eq!(fixture.factory.builds("z2m@server1"), 1);
    }

    #[tokio::test]
    async fn removed_server_stops_its_backend() {
        let mut fixture = Fixture::start().await;

        fixture.apply(|config| config.z2m.servers.clear()).await;

        assert!(!fixture.names().await.contains(&"z2m@server1".to_string()));
    }

    #[tokio::test]
    async fn added_server_starts_a_backend() {
        let mut fixture = Fixture::start().await;

        fixture
            .apply(|config| {
                let server = config.z2m.servers["server1"].clone();
                config.z2m.servers.insert("server2".into(), server);
            })
            .await;

        fixture.assert_running("z2m@server2").await;
        assert_eq!(fixture.factory.builds("z2m@server2"), 1);
        assert_eq!(fixture.factory.builds("z2m@server1"), 1);
    }

    #[tokio::test]
    async fn changed_server_restarts_its_backend() {
        let mut fixture = Fixture::start().await;

        fixture
            .apply(|config| {
                let server = config.z2m.servers.get_mut("server1").unwrap();
                server.group_prefix = Some("bifrost_".into());
            })
            .await;

        fixture.assert_running("z2m@server1").await;
        assert_eq!(fixture.factory.builds("z2m@server1"), 2);
    }

    #[tokio::test]
    async fn stop_and_restart_cycle() {
        let mut fixture = Fixture::start().await;
        let original = fixture.store.active();

        fixture.apply(|config| config.z2m.servers.clear()).await;
        fixture
            .apply(|config| config.z2m.servers = original.z2m.servers.clone())
            .await;

        fixture.assert_running("z2m@server1").await;
        assert_eq!(fixture.factory.builds("z2m@server1"), 2);
    }
}
