//! The certificate panel under `/bifrost/bridge/certificate`.
//!
//! Read-only: the panel shows what the bridge's own certificate says, and
//! nothing else. Regenerating it invalidates every paired client, and pairing
//! again is not possible before wave 6, so the endpoint exists but no screen
//! calls it yet.

use axum::Router;
use axum::extract::State;
use axum::routing::{get, post};

use bifrost_api::admin::CertificateInfo;

use crate::config::diff::HTTPS_SERVICE;
use crate::routes::bifrost::BifrostApiResult;
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;
use crate::server::certificate;
use crate::service_registry::{AppServiceFactory, ServiceFactory};

#[allow(clippy::result_large_err)]
async fn get_certificate(State(state): State<AppState>) -> BifrostApiResult<Json<CertificateInfo>> {
    let config = state.config();
    let info = certificate::describe(&state.store().paths().cert, config.bridge.mac)?;
    Ok(Json(info))
}

#[allow(clippy::result_large_err)]
async fn post_regenerate(State(state): State<AppState>) -> BifrostApiResult<Json<CertificateInfo>> {
    let config = state.config();
    let info = certificate::regenerate(&state.store().paths().cert, config.bridge.mac)?;
    log::warn!("The bridge certificate was regenerated; every paired client must pair again");

    // the https listener loaded the old certificate when it started
    let mut mgr = state.manager();
    if mgr.resolve(HTTPS_SERVICE).await.is_ok() {
        let svc = AppServiceFactory::new(state.clone()).build(HTTPS_SERVICE, &config)?;
        mgr.restart_with(HTTPS_SERVICE, svc).await?;
    } else {
        log::debug!("No https service is running; the new certificate applies at the next start");
    }

    Ok(Json(info))
}

/// The certificate routes, mounted by the `/bifrost` nest and therefore
/// behind the same guard as every other admin route.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_certificate))
        .route("/regenerate", post(post_regenerate))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use hyper::StatusCode;
    use serde_json::Value;
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::certificate::router;
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    async fn fixture() -> (TempDir, Router) {
        let dir = TempDir::new();
        let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();
        let router = router().with_state(state);
        (dir, router)
    }

    async fn call(router: &Router, method: Method, uri: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// The bridge id for `00:11:22:33:44:55`: the MAC with `ff:fe` inserted.
    const BRIDGE_ID: &str = "001122fffe334455";

    #[tokio::test]
    async fn panel_reports_the_generated_certificate() {
        let (_dir, router) = fixture().await;

        let (status, body) = call(&router, Method::GET, "/").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["common_name"], BRIDGE_ID);
        assert_eq!(body["bridge_id"], BRIDGE_ID);
        assert_eq!(body["mac"], "00:11:22:33:44:55");
        assert_eq!(body["not_before"], "2017-01-01T00:00:00Z");
        assert_eq!(body["not_after"], "2038-01-19T03:14:07Z");

        // SHA-256: 32 bytes, uppercase hex, colon separated
        let fingerprint = body["fingerprint"].as_str().unwrap();
        assert_eq!(fingerprint.split(':').count(), 32);
        assert!(fingerprint.starts_with(|c: char| c.is_ascii_hexdigit()));
        assert_eq!(fingerprint.to_uppercase(), fingerprint);
    }

    #[tokio::test]
    async fn regenerate_replaces_the_certificate() {
        let (_dir, router) = fixture().await;
        let (_, before) = call(&router, Method::GET, "/").await;

        let (status, after) = call(&router, Method::POST, "/regenerate").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(after["common_name"], BRIDGE_ID);
        // the serial is derived from the MAC, so only the key changes
        assert_eq!(after["serial"], before["serial"]);
        assert_ne!(after["fingerprint"], before["fingerprint"]);

        // the panel reads the file again, so it shows the new certificate
        let (_, again) = call(&router, Method::GET, "/").await;
        assert_eq!(again["fingerprint"], after["fingerprint"]);
    }

    #[tokio::test]
    async fn the_certificate_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, router) = fixture().await;
        call(&router, Method::POST, "/regenerate").await;

        let mode = std::fs::metadata(dir.path().join("cert.pem"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
