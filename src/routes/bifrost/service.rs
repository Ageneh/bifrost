//! The services table under `/bifrost/service`.
//!
//! Every service the manager runs, its state and, when it failed, why.
//! Restarting is stop and start on the same instance: the name, and with it
//! a `z2m@name` backend's configuration, stays as it is.

use std::collections::BTreeMap;

use axum::Router;
use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use uuid::Uuid;

use bifrost_api::service::{Service, ServiceList};
use svc::manager::SvmClient;
use svc::traits::ServiceState;

use crate::routes::bifrost::BifrostApiResult;
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;

async fn describe(mgr: &mut SvmClient, id: Uuid) -> BifrostApiResult<Service> {
    let name = mgr.lookup_name(id).await?;
    let state = mgr.status(id).await?;
    let last_error = mgr.last_error(id).await?;

    Ok(Service {
        id,
        name,
        state,
        last_error,
    })
}

async fn service_list(state: &AppState) -> BifrostApiResult<ServiceList> {
    let mut svm = state.manager();

    let mut services = BTreeMap::new();
    for (id, name) in svm.list().await? {
        let state = svm.status(id).await?;
        let last_error = svm.last_error(id).await?;

        let service = Service {
            id,
            name,
            state,
            last_error,
        };
        services.insert(id, service);
    }

    Ok(ServiceList { services })
}

async fn get_services(State(state): State<AppState>) -> BifrostApiResult<Json<ServiceList>> {
    Ok(Json(service_list(&state).await?))
}

async fn put_service(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(service_state): Json<ServiceState>,
) -> BifrostApiResult<Json<Uuid>> {
    let mut mgr = state.manager();

    let uuid = match service_state {
        ServiceState::Registered
        | ServiceState::Configured
        | ServiceState::Starting
        | ServiceState::Stopping
        | ServiceState::Failed => mgr.resolve(id).await?,
        ServiceState::Running => mgr.start(id).await?,
        ServiceState::Stopped => mgr.stop(id).await?,
    };

    Ok(Json(uuid))
}

/// Stop and start the service again, on the same instance.
///
/// Waits for the stop, so the port (or the z2m connection) is free before
/// the start; a service that never stops fails the request rather than
/// leaving two of them running.
async fn post_restart(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> BifrostApiResult<Json<Service>> {
    let mut mgr = state.manager();

    mgr.stop(id).await?;
    mgr.wait_for_stop(id).await?;
    mgr.start(id).await?;

    Ok(Json(describe(&mut mgr, id).await?))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_services))
        .route("/{id}", put(put_service))
        .route("/{id}/restart", post(post_restart))
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use hyper::StatusCode;
    use serde_json::Value;
    use svc::manager::ServiceManager;
    use svc::traits::Service;
    use tower::ServiceExt;

    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::service::router;
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    /// Runs until it is stopped.
    struct Idle;

    #[async_trait::async_trait]
    impl Service for Idle {
        type Error = Infallible;

        async fn run(&mut self) -> Result<(), Infallible> {
            futures::future::pending().await
        }
    }

    /// Fails at once, so the manager records a reason.
    struct Broken;

    #[async_trait::async_trait]
    impl Service for Broken {
        type Error = std::io::Error;

        async fn run(&mut self) -> Result<(), std::io::Error> {
            Err(std::io::Error::other(
                "the z2m server refused the connection",
            ))
        }
    }

    async fn fixture() -> (TempDir, AppState, Router) {
        let dir = TempDir::new();
        let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();
        let router = router().with_state(state.clone());
        (dir, state, router)
    }

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
            .header("content-type", "application/json")
            .body(body)
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Register `svc` as `name`, start it and wait until it runs.
    async fn running(state: &AppState, name: &str, svc: impl Service + 'static) -> uuid::Uuid {
        let mut mgr = state.manager();
        let id = mgr.register_service(name, svc).await.unwrap();
        mgr.start(id).await.unwrap();
        mgr.wait_for_start(id).await.unwrap();
        id
    }

    #[tokio::test]
    async fn list_reports_state_and_no_error_for_a_healthy_service() {
        let (_dir, state, router) = fixture().await;
        let id = running(&state, "idle", Idle).await;

        let (status, body) = call(&router, Method::GET, "/", None).await;

        assert_eq!(status, StatusCode::OK);
        let service = &body["services"][id.to_string()];
        assert_eq!(service["name"], "idle");
        assert_eq!(service["state"], "Running");
        assert_eq!(service["last_error"], Value::Null);
    }

    #[tokio::test]
    async fn list_reports_why_a_service_failed() {
        let (_dir, state, router) = fixture().await;
        let mut mgr = state.manager();
        let id = mgr.register_service("broken", Broken).await.unwrap();
        mgr.start(id).await.unwrap();
        drop(mgr);

        // the run policy retries once after a second before failing
        tokio::time::sleep(Duration::from_millis(2500)).await;

        let (_, body) = call(&router, Method::GET, "/", None).await;
        let service = &body["services"][id.to_string()];
        assert_eq!(service["state"], "Failed");
        assert_eq!(
            service["last_error"],
            "the z2m server refused the connection"
        );
    }

    #[tokio::test]
    async fn restart_stops_and_starts_the_same_instance() {
        let (_dir, state, router) = fixture().await;
        let id = running(&state, "idle", Idle).await;

        let (status, body) = call(
            &router,
            Method::POST,
            &format!("/{id}/restart"),
            Some(serde_json::json!({})),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], id.to_string());
        assert_eq!(body["state"], "Running");

        // the same instance, so the name still resolves to it
        let mut mgr = state.manager();
        assert_eq!(mgr.resolve(id).await.unwrap(), id);
        assert_eq!(
            mgr.status(id).await.unwrap(),
            svc::traits::ServiceState::Running
        );
    }

    #[tokio::test]
    async fn restart_of_an_unknown_service_is_refused() {
        let (_dir, _state, router) = fixture().await;
        let id = uuid::Uuid::nil();

        let (status, body) = call(
            &router,
            Method::POST,
            &format!("/{id}/restart"),
            Some(serde_json::json!({})),
        )
        .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            body["error"].as_str().unwrap().contains("not registered"),
            "{body}"
        );
    }
}
