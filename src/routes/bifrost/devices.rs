//! The device inventory under `/bifrost/devices`.
//!
//! A refusal is an HTTP response with the reason in it, which is a larger
//! error than clippy likes; every admin route here is written that way.
#![allow(clippy::result_large_err)]
//!
//! Reads come straight from the registry every z2m backend publishes into,
//! so the list is what the bridge knows right now and not what it knew when
//! a backend last connected. The one write is identify, which flashes a
//! device so a person can tell which bulb it is; it is addressed to one
//! server and never broadcast.

use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use hyper::StatusCode;
use serde::Deserialize;

use bifrost_api::inventory::{DeviceInfo, DeviceType, IdentifyRequest, InventorySnapshot};

use crate::backend::z2m::Z2mCommand;
use crate::routes::bifrost::refuse;
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;

/// How long an identify waits for the backend to write it to the socket.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(5);

/// The filters `GET /devices` accepts.
#[derive(Debug, Default, Deserialize)]
struct DeviceQuery {
    /// Only the devices of this z2m server.
    server: Option<String>,
    /// Only devices of this class.
    #[serde(rename = "type")]
    device_type: Option<String>,
    /// Only devices Bifrost can, or cannot, serve.
    supported: Option<bool>,
}

async fn list_devices(
    State(state): State<AppState>,
    Query(query): Query<DeviceQuery>,
) -> Result<Json<InventorySnapshot>, Response> {
    let mut snapshot = state.inventory().snapshot();

    if let Some(server) = &query.server {
        snapshot.servers.retain(|entry| &entry.name == server);
        snapshot.devices.retain(|device| &device.server == server);
    }

    if let Some(name) = &query.device_type {
        let Some(device_type) = DeviceType::parse(name) else {
            return Err(refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!("No device type is called {name:?}"),
            ));
        };
        snapshot
            .devices
            .retain(|device| device.device_type == device_type);
    }

    if let Some(supported) = query.supported {
        snapshot
            .devices
            .retain(|device| device.supported == supported);
    }

    Ok(Json(snapshot))
}

/// The one device with `ieee`, or a refusal that says what to do about it.
fn one_device(state: &AppState, ieee: &str, server: Option<&str>) -> Result<DeviceInfo, Response> {
    let mut candidates = state.inventory().devices_by_ieee(ieee);
    if let Some(server) = server {
        candidates.retain(|device| device.server == server);
    }

    match candidates.len() {
        0 => Err(refuse(
            StatusCode::NOT_FOUND,
            &format!("No device has the address {ieee:?}"),
        )),
        1 => Ok(candidates.remove(0)),
        _ => {
            let servers: Vec<&str> = candidates
                .iter()
                .map(|device| device.server.as_str())
                .collect();
            Err(refuse(
                StatusCode::CONFLICT,
                &format!("{ieee:?} is on several servers ({servers:?}); name one with ?server="),
            ))
        }
    }
}

async fn get_device(
    State(state): State<AppState>,
    Path(ieee): Path<String>,
    Query(query): Query<DeviceQuery>,
) -> Result<Json<DeviceInfo>, Response> {
    Ok(Json(one_device(&state, &ieee, query.server.as_deref())?))
}

async fn identify_device(
    State(state): State<AppState>,
    Path(ieee): Path<String>,
    Query(query): Query<DeviceQuery>,
    Json(request): Json<IdentifyRequest>,
) -> Result<Json<DeviceInfo>, Response> {
    let device = one_device(&state, &ieee, query.server.as_deref())?;

    let Some(sender) = state.backends().sender(&device.server) else {
        return Err(refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!(
                "The z2m server {:?} is not running, so nothing can be sent to it",
                device.server
            ),
        ));
    };

    let (reply, answer) = tokio::sync::oneshot::channel();
    let command = Z2mCommand::Identify {
        device: device.friendly_name.clone(),
        seconds: request.seconds.unwrap_or(5),
        reply,
    };

    if sender.send(command).is_err() {
        return Err(refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!(
                "The z2m server {:?} stopped accepting commands",
                device.server
            ),
        ));
    }

    match tokio::time::timeout(IDENTIFY_TIMEOUT, answer).await {
        Ok(Ok(Ok(()))) => Ok(Json(device)),
        Ok(Ok(Err(err))) => Err(refuse(StatusCode::BAD_GATEWAY, &err.to_string())),
        Ok(Err(_)) => Err(refuse(
            StatusCode::BAD_GATEWAY,
            "The backend did not answer the identify",
        )),
        Err(_) => Err(refuse(
            StatusCode::GATEWAY_TIMEOUT,
            "The backend did not take the identify in time",
        )),
    }
}

/// The device routes, mounted by the `/bifrost` nest and therefore behind the
/// same guard as every other admin route.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_devices))
        .route("/{ieee}", get(get_device))
        .route("/{ieee}/identify", post(identify_device))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use serde_json::Value;
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use bifrost_api::inventory::{Availability, Capabilities, DeviceInfo, DeviceType};

    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::devices::router;
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    fn device(server: &str, ieee: &str, name: &str, device_type: DeviceType) -> DeviceInfo {
        DeviceInfo {
            server: server.to_string(),
            ieee: ieee.to_string(),
            friendly_name: name.to_string(),
            model: Some("LCT001".into()),
            vendor: Some("Signify".into()),
            description: None,
            device_type,
            supported: device_type == DeviceType::Light,
            unsupported_reason: (device_type != DeviceType::Light)
                .then(|| "not served".to_string()),
            exposes: vec![],
            capabilities: Capabilities::default(),
            availability: Availability::Online,
            resources: vec![],
            room: None,
            group: None,
            last_seen: None,
            battery: None,
            readings: vec![],
        }
    }

    struct Fixture {
        _dir: TempDir,
        state: AppState,
        router: Router,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = TempDir::new();
            let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
            let (svm, _future) = ServiceManager::spawn();
            let state = AppState::from_store(Arc::new(store), svm).await.unwrap();

            let inventory = state.inventory();
            inventory.server_up("server1");
            inventory.server_up("server2");
            inventory.upsert(device("server1", "0x01", "Lamp", DeviceType::Light));
            inventory.upsert(device("server1", "0x02", "Plug", DeviceType::Plug));
            inventory.upsert(device("server2", "0x01", "Other lamp", DeviceType::Light));

            Self {
                _dir: dir,
                router: router().with_state(state.clone()),
                state,
            }
        }

        async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (u16, Value) {
            let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body)
                .unwrap();
            let response = self.router.clone().oneshot(request).await.unwrap();
            let status = response.status().as_u16();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            (
                status,
                serde_json::from_str(&text).unwrap_or(Value::String(text)),
            )
        }
    }

    #[tokio::test]
    async fn the_list_is_every_device_on_every_server() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture.call(Method::GET, "/", None).await;

        assert_eq!(status, 200);
        assert_eq!(body["devices"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn the_server_filter_narrows_to_one_server() {
        let fixture = Fixture::new().await;

        let (_, body) = fixture.call(Method::GET, "/?server=server2", None).await;

        let devices = body["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["friendly_name"], "Other lamp");
        assert_eq!(body["servers"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_type_and_supported_filters_combine() {
        let fixture = Fixture::new().await;

        let (_, body) = fixture.call(Method::GET, "/?type=light", None).await;
        assert_eq!(body["devices"].as_array().unwrap().len(), 2);

        let (_, body) = fixture
            .call(Method::GET, "/?type=light&supported=false", None)
            .await;
        assert_eq!(body["devices"].as_array().unwrap().len(), 0);

        let (_, body) = fixture.call(Method::GET, "/?type=plug", None).await;
        let devices = body["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["supported"], false);
        assert_eq!(devices[0]["unsupported_reason"], "not served");
    }

    #[tokio::test]
    async fn an_unknown_type_is_refused_with_the_reason() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture.call(Method::GET, "/?type=toaster", None).await;

        assert_eq!(status, 422);
        assert!(body["error"].as_str().unwrap().contains("toaster"));
    }

    #[tokio::test]
    async fn one_address_on_two_servers_needs_a_server_to_disambiguate() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture.call(Method::GET, "/0x01", None).await;
        assert_eq!(status, 409);
        assert!(body["error"].as_str().unwrap().contains("?server="));

        let (status, body) = fixture
            .call(Method::GET, "/0x01?server=server1", None)
            .await;
        assert_eq!(status, 200);
        assert_eq!(body["friendly_name"], "Lamp");
    }

    #[tokio::test]
    async fn an_unknown_address_is_not_found() {
        let fixture = Fixture::new().await;

        let (status, _) = fixture.call(Method::GET, "/0xff", None).await;

        assert_eq!(status, 404);
    }

    #[tokio::test]
    async fn identify_says_the_server_is_not_running_when_it_is_not() {
        let fixture = Fixture::new().await;

        let (status, body) = fixture
            .call(
                Method::POST,
                "/0x01/identify?server=server1",
                Some(serde_json::json!({})),
            )
            .await;

        assert_eq!(status, 503, "{body}");
        assert!(body["error"].as_str().unwrap().contains("not running"));
        assert_eq!(fixture.state.backends().running().len(), 0);
    }
}
