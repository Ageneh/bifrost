//! The stream limit the bridge advertises, and the state that carries it.
//!
//! The bridge's own `Entertainment` resource is persisted with the state, so
//! an install created before `bifrost.entertainment.max_streams` existed
//! keeps advertising whatever it was created with. The configuration wins:
//! the value is patched into the state on load.

mod support;

use std::sync::Arc;

use svc::manager::ServiceManager;

use bifrost::config_store::ConfigStore;
use bifrost::server::appstate::AppState;
use hue::api::{Entertainment, RType};
use support::temp_dir::TempDir;

/// A config with `max_streams` set, as an owner would write it.
fn config(max_streams: u32) -> String {
    format!(
        "\
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 127.0.0.1
  netmask: 255.0.0.0
  gateway: 127.0.0.1
  timezone: UTC

z2m: {{}}

bifrost:
  entertainment:
    max_streams: {max_streams}
"
    )
}

/// Boot on `dir` with the config in place and answer with the advertised
/// limit.
async fn boot(dir: &TempDir, max_streams: u32) -> (AppState, Arc<ConfigStore>) {
    let store =
        Arc::new(ConfigStore::load(&dir.write("config.yaml", &config(max_streams))).unwrap());
    let (svm, _future) = ServiceManager::spawn();
    let state = AppState::from_store(store.clone(), svm).await.unwrap();
    (state, store)
}

/// The `max_streams` the bridge's entertainment resource carries.
async fn advertised(state: &AppState) -> Option<u32> {
    let res = state.res.lock().await;

    res.get_resource_ids_by_type(RType::Entertainment)
        .into_iter()
        .filter_map(|id| res.get_id::<Entertainment>(id).ok())
        .find_map(|ent| ent.max_streams)
}

/// A state written by an older version carries `1`; the configuration
/// decides, so a fresh install and an upgraded one agree.
#[tokio::test]
async fn an_existing_state_is_patched_from_the_configuration() {
    let dir = TempDir::new();

    // an install created when one stream was all there was
    let (state, store) = boot(&dir, 1).await;
    assert_eq!(advertised(&state).await, Some(1));

    // write that state out, the way the config writer would
    let yaml = state.res.lock().await.serialize().unwrap();
    std::fs::write(store.paths().state.as_std_path(), yaml).unwrap();
    drop(state);

    // the same install, now configured for two
    let (state, _store) = boot(&dir, 2).await;
    assert_eq!(
        advertised(&state).await,
        Some(2),
        "the persisted value was not patched from the configuration"
    );
}

/// Two streams are the default, because that is the case the wave exists
/// for: a TV and a second client at once.
#[tokio::test]
async fn a_config_that_names_no_limit_advertises_two() {
    let dir = TempDir::new();
    let store = Arc::new(
        ConfigStore::load(&dir.write(
            "config.yaml",
            "\
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 127.0.0.1
  netmask: 255.0.0.0
  gateway: 127.0.0.1
  timezone: UTC

z2m: {}
",
        ))
        .unwrap(),
    );
    let (svm, _future) = ServiceManager::spawn();
    let state = AppState::from_store(store, svm).await.unwrap();

    assert_eq!(advertised(&state).await, Some(2));
}
