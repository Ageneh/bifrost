use std::collections::HashMap;
use std::fs::{self, File};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};

use camino::Utf8Path;
use chrono::Utc;
use tokio::sync::Mutex;
use tokio::sync::broadcast;

use bifrost_api::admin::{LinkButtonStatus, PairingMode, PairingRecord};
use hue::legacy_api::{ApiConfig, ApiShortConfig, Whitelist};
use svc::manager::SvmClient;

use crate::auth::{Auth, Clock, HashCost, SystemClock};
use crate::backend::z2m::BackendCommands;
use crate::config::AppConfig;
use crate::config_store::ConfigStore;
use crate::error::ApiResult;
use crate::inventory::Inventory;
use crate::model::state::{State, StateVersion};
use crate::resource::Resources;
use crate::server::certificate;
use crate::server::streams::StreamRegistry;
use crate::server::updater::VersionUpdater;

/// Seconds a pairing window stays open when the request names none.
pub const DEFAULT_WINDOW_SECONDS: u64 = 30;

/// Longest pairing window the API accepts.
pub const MAX_WINDOW_SECONDS: u64 = 120;

/// Pairing attempts the bridge remembers. The history is a log for the
/// screen, not an access list, so it is bounded rather than persisted.
const HISTORY_CAPACITY: usize = 50;

/// How many sockets may lag behind the pairing updates.
const EVENT_BUFFER: usize = 16;

/// The stand-in for the bridge's physical link button.
///
/// The window is a deadline and nothing else: every read compares it with
/// the clock, so it expires without a timer. The deadline sits in an
/// `AtomicU64` of Unix milliseconds — `0` means closed — so it is never held
/// across an await and two pairing requests cannot both win it.
pub struct LinkButtonState {
    clock: Arc<dyn Clock>,
    deadline_millis: AtomicU64,
    history: StdMutex<Vec<PairingRecord>>,
    events: broadcast::Sender<LinkButtonStatus>,
}

impl LinkButtonState {
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            clock,
            deadline_millis: AtomicU64::new(0),
            history: StdMutex::new(Vec::new()),
            events,
        }
    }

    fn now_millis(&self) -> u64 {
        u64::try_from(self.clock.now().timestamp_millis()).unwrap_or(0)
    }

    /// Open a window of `seconds`, replacing one that is already open.
    pub fn press(&self, seconds: u64) -> LinkButtonStatus {
        let deadline = self
            .now_millis()
            .saturating_add(seconds.saturating_mul(1000));
        self.deadline_millis.store(deadline, Ordering::SeqCst);

        let status = LinkButtonStatus {
            pressed: true,
            remaining_seconds: seconds,
        };
        let _ = self.events.send(status);
        status
    }

    /// The window as the clock sees it right now.
    #[must_use]
    pub fn status(&self) -> LinkButtonStatus {
        let deadline = self.deadline_millis.load(Ordering::SeqCst);
        let now = self.now_millis();
        if deadline == 0 || now >= deadline {
            return LinkButtonStatus {
                pressed: false,
                remaining_seconds: 0,
            };
        }

        // round up, so a 5 second window never reports 4
        LinkButtonStatus {
            pressed: true,
            remaining_seconds: deadline.saturating_sub(now).div_ceil(1000),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.status().pressed
    }

    /// Close the window, and report whether it was open.
    ///
    /// A successful pairing consumes the window, so a second client in the
    /// same window is refused — the behaviour a real bridge has.
    pub fn take(&self) -> bool {
        let deadline = self.deadline_millis.load(Ordering::SeqCst);
        if deadline == 0 || self.now_millis() >= deadline {
            return false;
        }
        if self
            .deadline_millis
            .compare_exchange(deadline, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            // another request consumed it first
            return false;
        }

        let _ = self.events.send(LinkButtonStatus {
            pressed: false,
            remaining_seconds: 0,
        });
        true
    }

    /// Append a pairing attempt to the history, newest first.
    pub fn record(&self, devicetype: &str, accepted: bool) {
        let record = PairingRecord {
            // client-supplied and unauthenticated: keep it short enough to
            // render and to store
            devicetype: truncate(devicetype, 64),
            at: self
                .clock
                .now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            accepted,
        };

        let mut history = self.history.lock().unwrap_or_else(PoisonError::into_inner);
        history.insert(0, record);
        history.truncate(HISTORY_CAPACITY);
    }

    #[must_use]
    pub fn history(&self) -> Vec<PairingRecord> {
        self.history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replace the history, which an identity restore does.
    pub fn set_history(&self, records: Vec<PairingRecord>) {
        let mut history = self.history.lock().unwrap_or_else(PoisonError::into_inner);
        *history = records;
        history.truncate(HISTORY_CAPACITY);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LinkButtonStatus> {
        self.events.subscribe()
    }
}

/// `text` cut to `max` characters, on a character boundary.
fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[derive(Clone)]
pub struct AppState {
    store: Arc<ConfigStore>,
    upd: Arc<Mutex<VersionUpdater>>,
    svm: SvmClient,
    auth: Arc<Auth>,
    pub res: Arc<Mutex<Resources>>,
    inventory: Arc<Inventory>,
    backends: Arc<BackendCommands>,
    link_button: Arc<LinkButtonState>,
    streams: Arc<StreamRegistry>,
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

        // a MAC change is only picked up at startup, and the state was
        // written for the old bridge id: rebuild the bridge resources around
        // the new one, keeping rooms, scenes and imported devices
        let bridge_id = hue::bridge_id(config.bridge.mac);
        res.adopt_bridge_id(&bridge_id)?;

        res.reset_all_streaming()?;

        // the bridge's entertainment resource is persisted with the state,
        // and it is what a client reads to decide how many streams it may
        // open: an install created before this setting existed would keep
        // advertising one forever
        res.patch_max_streams(config.bifrost.entertainment.max_streams)?;

        let res = Arc::new(Mutex::new(res));

        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let auth = Arc::new(Auth::load(
            &store.paths().sessions,
            clock.clone(),
            HashCost::DEFAULT,
        ));
        if config.bifrost.auth.password_hash.is_none() {
            // deleting the password is how a lost one is reset, and that
            // must log out everyone who was logged in with it
            auth.revoke_all()?;
        }

        Ok(Self {
            store,
            upd,
            svm,
            auth,
            res,
            inventory: Inventory::new(),
            backends: Arc::new(BackendCommands::default()),
            link_button: Arc::new(LinkButtonState::new(clock)),
            streams: Arc::new(StreamRegistry::new()),
        })
    }

    /// The link-button stand-in the pairing screen drives.
    #[must_use]
    pub fn link_button(&self) -> Arc<LinkButtonState> {
        self.link_button.clone()
    }

    /// Whether `POST /api` needs an open pairing window.
    #[must_use]
    pub fn pairing_mode(&self) -> PairingMode {
        PairingMode::from_required(self.config().pairing.require_link_button)
    }

    /// Replace the pairing window's clock, e.g. with one on a test clock.
    #[must_use]
    pub fn with_link_button_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.link_button = Arc::new(LinkButtonState::new(clock));
        self
    }

    /// The entertainment streams that are running.
    #[must_use]
    pub fn streams(&self) -> Arc<StreamRegistry> {
        self.streams.clone()
    }

    /// The device registry every z2m backend publishes into.
    #[must_use]
    pub fn inventory(&self) -> Arc<Inventory> {
        self.inventory.clone()
    }

    /// The command channels of the running z2m backends.
    #[must_use]
    pub fn backends(&self) -> Arc<BackendCommands> {
        self.backends.clone()
    }

    /// Replace the authentication state, e.g. with one on a test clock.
    #[must_use]
    pub fn with_auth(mut self, auth: Arc<Auth>) -> Self {
        self.auth = auth;
        self
    }

    #[must_use]
    pub fn auth(&self) -> Arc<Auth> {
        self.auth.clone()
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
        let mut short =
            ApiShortConfig::from_mac_and_version(mac, self.upd.lock().await.get().await);
        // a real bridge reports its physical button; Bifrost reports the
        // window the admin screen opened in its place
        short.linkbutton = self.link_button.is_open();
        short
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
