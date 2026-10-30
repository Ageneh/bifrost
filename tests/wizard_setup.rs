//! The first-run wizard's record, and what it asks zigbee2mqtt.
//!
//! The record lives beside the configuration draft and is private like it,
//! so a half-finished setup survives a restart and is never mistaken for
//! unapplied edits. Discovery is the one question the wizard asks a server
//! before a bridge exists.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request};
use hyper::StatusCode;
use serde_json::{Value, json};
use svc::manager::ServiceManager;
use tower::ServiceExt;

use bifrost::config_store::ConfigStore;
use bifrost::routes::bifrost::wizard;
use bifrost::server::appstate::AppState;
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

const CONFIG: &str = "\
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 127.0.0.1
  netmask: 255.0.0.0
  gateway: 127.0.0.1
  timezone: UTC

z2m:
  server1:
    url: ws://127.0.0.1:1
";

async fn call(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(body)
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test]
async fn the_wizard_record_survives_a_restart() {
    let dir = TempDir::new();
    let path = dir.write("config.yaml", CONFIG);
    let store = Arc::new(ConfigStore::load(&path).unwrap());
    let (svm, _future) = ServiceManager::spawn();
    let state = AppState::from_store(store.clone(), svm).await.unwrap();
    let admin = wizard::router().with_state(state);

    // nothing written yet: the wizard starts at the first step
    let (status, body) = call(&admin, Method::GET, "/wizard", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["step"], "identity");
    assert_eq!(body["values"], json!({}));

    let (status, body) = call(
        &admin,
        Method::PUT,
        "/wizard",
        Some(json!({
            "step": "lights",
            "values": {"bridge": {"name": "Attic"}, "devices": {"0x0017880100000001": {"enabled": true}}},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["step"], "lights");

    // the record is its own file, private, and not the configuration draft
    let wizard_file = store.paths().wizard.clone();
    assert!(wizard_file.is_file(), "{wizard_file} is missing");
    assert!(!store.paths().draft.exists(), "the wizard wrote the draft");
    let mode = std::fs::metadata(&wizard_file)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "{wizard_file} is not private");

    // a fresh process reads the same step back
    let reloaded = Arc::new(ConfigStore::load(&path).unwrap());
    let (svm, _future) = ServiceManager::spawn();
    let state = AppState::from_store(reloaded, svm).await.unwrap();
    let (_, body) = call(
        &wizard::router().with_state(state),
        Method::GET,
        "/wizard",
        None,
    )
    .await;

    assert_eq!(body["step"], "lights");
    assert_eq!(body["values"]["bridge"]["name"], "Attic");
}

#[tokio::test]
async fn discovery_reads_the_devices_and_rooms_a_server_announces() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let server: bifrost::config::Z2mServer = serde_json::from_value(json!({
        "url": mock.url(),
    }))
    .unwrap();

    let (devices, rooms) = bifrost::backend::z2m::discover::devices(&server)
        .await
        .unwrap();

    // the coordinator is not a device the wizard can serve
    let names: Vec<&str> = devices
        .iter()
        .map(|device| device.friendly_name.as_str())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(!names.contains(&"Coordinator"), "{names:?}");
    assert!(devices.iter().all(|device| device.supported), "{devices:?}");

    // and the group is the room the wizard offers
    assert_eq!(rooms.len(), 1, "{rooms:?}");
    assert_eq!(rooms[0].topic, "Kitchen");
    assert_eq!(rooms[0].members.len(), 2, "{rooms:?}");
}
