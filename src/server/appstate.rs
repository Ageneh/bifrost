use std::collections::HashMap;
use std::fs::{self, File};
use std::sync::Arc;

use camino::Utf8Path;
use chrono::Utc;
use tokio::sync::Mutex;

use hue::legacy_api::{ApiConfig, ApiShortConfig, Whitelist};
use svc::manager::SvmClient;

use crate::config::AppConfig;
use crate::config_store::ConfigStore;
use crate::error::ApiResult;
use crate::model::state::{State, StateVersion};
use crate::resource::Resources;
use crate::server::certificate;
use crate::server::updater::VersionUpdater;

#[derive(Clone)]
pub struct AppState {
    store: Arc<ConfigStore>,
    upd: Arc<Mutex<VersionUpdater>>,
    svm: SvmClient,
    pub res: Arc<Mutex<Resources>>,
}

impl AppState {
    pub async fn from_store(store: Arc<ConfigStore>, svm: SvmClient) -> ApiResult<Self> {
        let config = store.active();
        let certfile = &store.paths().cert;
        let statefile = &store.paths().state;

        let certpath = Utf8Path::new(certfile);
        if certpath.is_file() {
            certificate::check_certificate(certpath, config.bridge.mac)?;
        } else {
            log::warn!("Missing certificate file [{certfile}], generating..");
            certificate::generate_and_save(certpath, config.bridge.mac)?;
        }

        let mut res;
        let upd = Arc::new(Mutex::new(VersionUpdater::with_default_version()));
        let swversion = upd.lock().await.get().await.clone();

        if let Ok(fd) = File::open(statefile) {
            log::debug!("Existing state file found, loading..");
            let yaml = serde_yml::from_reader(fd)?;
            let state = match State::version(&yaml)? {
                StateVersion::V0 => {
                    log::info!("Detected state file version 0. Upgrading to new version..");
                    let backup_path = &statefile.with_extension("v0.bak");
                    fs::rename(statefile, backup_path)?;
                    log::info!("  ..saved old state file as {backup_path}");
                    State::from_v0(yaml)?
                }
                StateVersion::V1 => {
                    log::info!("Detected state file version 1. Loading..");
                    State::from_v1(yaml)?
                }
            };
            res = Resources::new(swversion, state);
        } else {
            log::debug!("No state file found, initializing..");
            res = Resources::new(swversion, State::new());
            res.init(&hue::bridge_id(config.bridge.mac))?;
        }

        res.reset_all_streaming()?;

        let res = Arc::new(Mutex::new(res));

        Ok(Self {
            store,
            upd,
            svm,
            res,
        })
    }

    /// Snapshot of the active configuration. Callers holding the snapshot
    /// keep seeing it after an apply; read it again to see changes.
    #[must_use]
    pub fn config(&self) -> Arc<AppConfig> {
        Arc::new(self.store.active())
    }

    #[must_use]
    pub fn store(&self) -> Arc<ConfigStore> {
        self.store.clone()
    }

    #[must_use]
    pub fn updater(&self) -> Arc<Mutex<VersionUpdater>> {
        self.upd.clone()
    }

    #[must_use]
    pub fn manager(&self) -> SvmClient {
        self.svm.clone()
    }

    #[must_use]
    pub async fn api_short_config(&self) -> ApiShortConfig {
        let mac = self.config().bridge.mac;
        ApiShortConfig::from_mac_and_version(mac, self.upd.lock().await.get().await)
    }

    pub async fn api_config(&self, username: String) -> ApiResult<ApiConfig> {
        let conf = self.config();
        let tz = tzfile::Tz::named(&conf.bridge.timezone)?;
        let localtime = Utc::now().with_timezone(&&tz).naive_local();

        let res = ApiConfig {
            short_config: self.api_short_config().await,
            ipaddress: conf.bridge.ipaddress,
            netmask: conf.bridge.netmask,
            gateway: conf.bridge.gateway,
            timezone: conf.bridge.timezone.clone(),
            whitelist: HashMap::from([(
                username,
                Whitelist {
                    create_date: Utc::now(),
                    last_use_date: Utc::now(),
                    name: "User#foo".to_string(),
                },
            )]),
            localtime,
            ..ApiConfig::default()
        };

        Ok(res)
    }
}
