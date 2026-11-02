//! Room membership at apply, against the mock zigbee2mqtt.
//!
//! The Rooms screen drafts membership into `rooms.<topic>.devices`; apply
//! sends the difference to the group through the same request a v2 room
//! `PUT` sends, and names any room it could not reach. The backend, the
//! service registry and the admin API are the real ones; only the core
//! bridge services are stood in for, so the test binds no privileged port.

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request};
use futures::future::BoxFuture;
use hyper::StatusCode;
use serde_json::{Value, json};
use svc::manager::ServiceManager;
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};
use tower::ServiceExt;

use bifrost::config::AppConfig;
use bifrost::config::diff::ApplyAction;
use bifrost::config_store::ConfigStore;
use bifrost::error::ApiResult;
use bifrost::routes::bifrost::config::{ApplyHook, admin_router};
use bifrost::server::appstate::AppState;
use bifrost::service_registry::{self, AppServiceFactory, ServiceFactory};
use support::mock_z2m::MockZ2m;
use support::temp_dir::TempDir;

/// How long a test waits for the mesh and the import to catch up.
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
    state: AppState,
    admin: Router,
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
        let (mut svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(store.clone(), svm.clone())
            .await
            .unwrap();

        let factory = CoreIdleFactory {
            z2m: AppServiceFactory::new(state.clone()),
        };
        service_registry::reconcile(&store, &mut svm, &factory, &[])
            .await
            .unwrap();

        // the apply is driven through the admin API; the hook is the only
        // part of the restart stood in for
        let hook: ApplyHook = Arc::new(|_state: AppState, _actions: Vec<ApplyAction>| {
            Box::pin(async { Ok(()) }) as BoxFuture<'static, ApiResult<()>>
        });

        Self {
            _dir: dir,
            store,
            admin: admin_router(hook).with_state(state.clone()),
            state,
        }
    }

    /// Write `config` as the draft and apply it through the admin API.
    async fn apply(&self, config: AppConfig) -> Value {
        let revision = self.store.revision();
        self.store.set_draft(revision, config).unwrap();

        let (status, body) = call(
            &self.admin,
            Method::POST,
            "/apply",
            json!({"revision": revision + 1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The children of the room named `name`, as the bridge holds them.
    async fn room_children(&self, name: &str) -> Option<usize> {
        let res = self.state.res.lock().await;
        res.get_resource_ids_by_type(hue::api::RType::Room)
            .into_iter()
            .filter_map(|rid| res.get_id::<hue::api::Room>(rid).ok())
            .find(|room| room.metadata.name == name)
            .map(|room| room.children.len())
    }

    /// Wait until the mock's group has been imported as a room of `size`.
    async fn wait_for_room(&self, size: usize) {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            if self.room_children("Kitchen").await == Some(size) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "waited {PATIENCE:?} for a room of {size} device(s)"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn call(router: &Router, method: Method, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::from(body.to_string()))
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
async fn apply_sends_the_membership_difference_to_the_group() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let fixture = Fixture::start(mock.url()).await;
    fixture.wait_for_room(2).await;

    let mut config = fixture.store.active();
    config.rooms.insert(
        "Kitchen".into(),
        bifrost::config::RoomConfig {
            devices: Some(vec![
                "0x0017880100000001".into(),
                // the attic bulb was in no group
                "0x0017880100000003".into(),
            ]),
            ..bifrost::config::RoomConfig::default()
        },
    );

    let body = fixture.apply(config).await;
    // nothing was left undone, so the response names nothing
    assert!(body.get("failed").is_none_or(Value::is_null), "{body}");

    // the group lost the hall lamp and gained the attic bulb
    let added = mock.wait_for("bridge/request/group/members/add").await;
    assert_eq!(added["device"], "Attic bulb", "{added}");
    assert_eq!(added["group"], "Kitchen", "{added}");

    // both requests go out together, so neither is waited for twice
    let removed = mock.received_on("bridge/request/group/members/remove");
    assert!(
        removed.iter().any(|change| change["device"] == "Hall lamp"),
        "{removed:?}"
    );
}

#[tokio::test]
async fn a_room_that_cannot_be_reached_is_named() {
    let mock = MockZ2m::start_w05().await.unwrap();
    let fixture = Fixture::start(mock.url()).await;
    fixture.wait_for_room(2).await;

    let mut config = fixture.store.active();
    // this room is on the bridge, so its change goes through
    config.rooms.insert(
        "Kitchen".into(),
        bifrost::config::RoomConfig {
            devices: Some(vec!["0x0017880100000001".into()]),
            ..bifrost::config::RoomConfig::default()
        },
    );
    // this one has no group behind it
    config.rooms.insert(
        "Attic".into(),
        bifrost::config::RoomConfig {
            devices: Some(vec!["0x0017880100000003".into()]),
            ..bifrost::config::RoomConfig::default()
        },
    );

    let body = fixture.apply(config).await;
    let failed = body["failed"].as_array().unwrap();

    // the room that could not be reached is named, and only that one
    assert_eq!(failed.len(), 1, "{body}");
    assert_eq!(failed[0]["path"], "rooms.Attic", "{body}");

    // the other room still applied
    let removed = mock.wait_for("bridge/request/group/members/remove").await;
    assert_eq!(removed["device"], "Hall lamp", "{removed}");
}
