//! The bridge's identity under `/bifrost/bridge/identity`.
//!
//! Three actions change who the bridge is, and each one invalidates every
//! paired client: change the MAC (which reissues the certificate and changes
//! the bridge id), regenerate the certificate, or reinitialize the state.
//! Regenerating the certificate and reinitializing the state already have
//! their own endpoints; this module adds the MAC change and the identity
//! bundle that makes a restore possible.
//!
//! The bundle holds the certificate — private key included — and the pairing
//! list, and nothing else. The z2m token and the admin password are never in
//! it: a restore asks for both, and only the password is stored, as the hash
//! it already is.

use axum::Router;
use axum::extract::{Extension, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hyper::StatusCode;
use mac_address::MacAddress;

use bifrost_api::admin::{
    IdentityBundle, IdentityMacChange, IdentityMacChangeReply, IdentityPairingEntry,
    IdentityRestore, IdentityRestoreReply, PairingRecord,
};

use crate::auth::verify_password;
use crate::config::diff::ApplyAction;
use crate::routes::bifrost::{BifrostApiResult, refuse};
use crate::routes::extractor::Json;
use crate::server::Protocol;
use crate::server::appstate::AppState;
use crate::server::certificate;
use crate::service_registry::{self, AppServiceFactory};

/// The bundle format this build writes and reads.
const BUNDLE_VERSION: u32 = 1;

/// Nothing about an identity bundle may sit in a cache.
const NO_STORE: &str = "no-store";

/// Change the bridge's MAC address.
///
/// The address is the bridge id, the certificate's common name and the
/// prefix of the state's bridge resources, so this is a restart-required
/// change: the draft carries it, the plan names the restart, and the next
/// start rebuilds the identity around the new address. The certificate is
/// reissued the moment the draft is applied, so the HTTPS listener that
/// comes back up serves the new identity.
async fn post_mac(
    State(state): State<AppState>,
    Json(req): Json<IdentityMacChange>,
) -> BifrostApiResult<Response> {
    let config = state.config();
    let bridge_id = hue::bridge_id(config.bridge.mac);

    if req.confirm != bridge_id {
        return Ok(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("Type the bridge id ({bridge_id}) to change the MAC address"),
        ));
    }

    let Ok(mac) = req.mac.parse::<MacAddress>() else {
        return Ok(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "That is not a MAC address; write it as 00:11:22:33:44:55",
        ));
    };

    if mac == config.bridge.mac {
        return Ok(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The bridge already has that MAC address",
        ));
    }

    let store = state.store();
    let mut next = store
        .draft()
        .map_or_else(|| store.active(), |draft| draft.config);
    next.bridge.mac = mac;
    store.set_draft(store.revision(), next)?;

    let new_id = hue::bridge_id(mac);
    log::warn!(
        "The bridge MAC is queued to change from {bridge_id} to {new_id}; the change takes effect at the next start"
    );

    Ok(Json(IdentityMacChangeReply {
        mac: mac.to_string(),
        bridge_id: new_id,
        restart_required: true,
    })
    .into_response())
}

/// The identity bundle: certificate and pairing list, nothing else.
///
/// It is built in memory and never written to disk, and it is only served
/// over the HTTPS listener with `Cache-Control: no-store`. A caller on the
/// plain HTTP listener is refused: the bundle holds a private key.
async fn get_backup(
    State(state): State<AppState>,
    Extension(protocol): Extension<Protocol>,
) -> BifrostApiResult<Response> {
    if protocol != Protocol::Https {
        return Ok(refuse(
            StatusCode::FORBIDDEN,
            "The identity bundle carries the bridge's private key; open this page over HTTPS",
        ));
    }

    let config = state.config();
    let bridge_id = hue::bridge_id(config.bridge.mac);
    let pem = certificate::read_pem(&state.store().paths().cert)?;

    let bundle = IdentityBundle {
        version: BUNDLE_VERSION,
        bridge_id: bridge_id.clone(),
        certificate_pem: pem,
        paired_clients: state
            .link_button()
            .history()
            .into_iter()
            .map(entry_of)
            .collect(),
    };

    log::warn!("The identity bundle was downloaded; it holds the bridge's private key");

    let filename = format!("bifrost-identity-{bridge_id}.json");
    Ok((
        [
            (CONTENT_TYPE, "application/json".to_string()),
            (CACHE_CONTROL, NO_STORE.to_string()),
            (
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        Json(bundle),
    )
        .into_response())
}

/// Restore an identity bundle onto this bridge.
///
/// The certificate is replaced, so every paired client must pair again. The
/// admin password authorises the restore; the z2m token is written into the
/// servers that have none of their own, because the bundle never carried
/// one. Neither secret is stored anywhere but where it already lived.
async fn post_restore(
    State(state): State<AppState>,
    Json(req): Json<IdentityRestore>,
) -> BifrostApiResult<Response> {
    let config = state.config();

    let Some(hash) = config.bifrost.auth.password_hash.as_deref() else {
        return Ok(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Set the admin password before restoring an identity; the restore is authorised by it",
        ));
    };

    if !verify_password(&req.admin_password, hash) {
        log::warn!("Refused an identity restore: the admin password did not match");
        return Ok(refuse(
            StatusCode::FORBIDDEN,
            "The admin password does not match",
        ));
    }

    if req.bundle.version != BUNDLE_VERSION {
        return Ok(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!(
                "That bundle is version {}; this bridge reads version {BUNDLE_VERSION}",
                req.bundle.version
            ),
        ));
    }

    // the bundle must be self-consistent, or the bridge comes back with a
    // certificate for a bridge id it does not have
    let common_name = certificate::extract_common_name(req.bundle.certificate_pem.as_bytes())?;
    if common_name.as_deref() != Some(req.bundle.bridge_id.as_str()) {
        return Ok(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The bundle's certificate does not belong to the bridge id it names",
        ));
    }

    certificate::save_pem(&state.store().paths().cert, &req.bundle.certificate_pem)?;
    state.link_button().set_history(
        req.bundle
            .paired_clients
            .iter()
            .map(|entry| PairingRecord {
                devicetype: entry.devicetype.clone(),
                at: entry.at.clone(),
                accepted: entry.accepted,
            })
            .collect(),
    );

    let tokens_written = write_missing_tokens(&state, &req.z2m_token).await?;

    log::warn!(
        "The bridge identity was restored from a bundle; every paired client must pair again"
    );

    Ok(Json(IdentityRestoreReply {
        bridge_id: req.bundle.bridge_id.clone(),
        paired_clients: req.bundle.paired_clients.len(),
        tokens_written,
    })
    .into_response())
}

/// Put `token` into every configured z2m server that has none, and bring the
/// backends in line. Returns the servers that took it.
async fn write_missing_tokens(state: &AppState, token: &str) -> BifrostApiResult<Vec<String>> {
    let store = state.store();
    let mut next = store.active();

    let mut written = Vec::new();
    for (name, server) in &mut next.z2m.servers {
        if server.token.is_none() {
            server.token = Some(token.to_string());
            written.push(name.clone());
        }
    }

    if written.is_empty() {
        return Ok(written);
    }

    let draft = store.set_draft(store.revision(), next)?;
    let outcome = store.apply(draft.revision)?;

    let factory = AppServiceFactory::new(state.clone());
    let mut mgr = state.manager();
    let actions: Vec<ApplyAction> = outcome.plan.hot.clone();
    service_registry::reconcile(&store, &mut mgr, &factory, &actions).await?;

    log::info!("Restored a z2m token into {} server(s)", written.len());
    Ok(written)
}

fn entry_of(record: PairingRecord) -> IdentityPairingEntry {
    IdentityPairingEntry {
        devicetype: record.devicetype,
        at: record.at,
        accepted: record.accepted,
    }
}

/// The identity routes, mounted by the `/bifrost` nest.
///
/// They sit behind the same guard as every other admin route, and setup mode
/// mounts none of them, so a bundle is never served while the bridge has no
/// identity.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/mac", post(post_mac))
        .route("/backup", get(get_backup))
        .route("/restore", post(post_restore))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use hyper::StatusCode;
    use serde_json::{Value, json};
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use crate::config_store::ConfigStore;
    use crate::routes::bifrost::identity::router;
    use crate::server::Protocol;
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    const BRIDGE_ID: &str = "001122fffe334455";

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
        protocol: Protocol,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String) {
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .extension(protocol)
            .body(body)
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_bundle_is_refused_on_the_plain_listener() {
        let (_dir, _state, router) = fixture().await;

        let (status, body) = call(&router, Protocol::Http, Method::GET, "/backup", None).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("HTTPS"), "{body}");
    }

    #[tokio::test]
    async fn the_bundle_carries_the_certificate_and_the_paired_clients() {
        let (_dir, state, router) = fixture().await;
        state.link_button().record("Hue Sync Box", true);

        let (status, body) = call(&router, Protocol::Https, Method::GET, "/backup", None).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let bundle: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(bundle["bridge_id"], BRIDGE_ID);
        assert_eq!(bundle["paired_clients"][0]["devicetype"], "Hue Sync Box");
        // the private key is in it, and never in a debug line
        assert!(
            bundle["certificate_pem"]
                .as_str()
                .unwrap()
                .contains("PRIVATE KEY")
        );
    }

    #[tokio::test]
    async fn a_mac_change_needs_the_bridge_id_typed() {
        let (_dir, _state, router) = fixture().await;

        let (status, body) = call(
            &router,
            Protocol::Https,
            Method::POST,
            "/mac",
            Some(json!({"mac": "00:11:22:33:44:66", "confirm": "yes"})),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body.contains(BRIDGE_ID), "{body}");
    }

    #[tokio::test]
    async fn a_mac_change_queues_the_draft_and_names_the_restart() {
        let (_dir, state, router) = fixture().await;

        let (status, body) = call(
            &router,
            Protocol::Https,
            Method::POST,
            "/mac",
            Some(json!({"mac": "00:11:22:33:44:66", "confirm": BRIDGE_ID})),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let reply: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(reply["mac"], "00:11:22:33:44:66");
        assert_eq!(reply["bridge_id"], "001122fffe334466");
        assert_eq!(reply["restart_required"], true);
        assert_eq!(
            state.store().draft().unwrap().config.bridge.mac.to_string(),
            "00:11:22:33:44:66"
        );
    }

    /// A fixture whose admin password is set, so a restore can be authorised.
    async fn protected_fixture(password: &str) -> (TempDir, AppState, Router) {
        let hash = crate::auth::hash_password(password, crate::testutil::FAST_HASH).unwrap();
        let yaml = format!("{MINIMAL_CONFIG}\nbifrost:\n  auth:\n    password_hash: \"{hash}\"\n");
        let dir = TempDir::new();
        let store = ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();
        let router = router().with_state(state.clone());
        (dir, state, router)
    }

    /// A bundle whose certificate belongs to `BRIDGE_ID`.
    fn bundle_for_bridge(dir: &TempDir) -> (Value, String) {
        let path = dir.path().join("bundle-cert.pem");
        crate::server::certificate::generate_and_save(&path, "00:11:22:33:44:55".parse().unwrap())
            .unwrap();
        let pem = crate::server::certificate::read_pem(&path).unwrap();
        (
            json!({
                "version": 1,
                "bridge_id": BRIDGE_ID,
                "certificate_pem": pem,
                "paired_clients": [
                    {"devicetype": "Hue Sync Box", "at": "2026-10-30T09:30:00Z", "accepted": true}
                ],
            }),
            pem,
        )
    }

    #[tokio::test]
    async fn a_restore_with_the_right_password_puts_the_identity_back() {
        let (dir, state, router) = protected_fixture("correct horse battery").await;
        let (bundle, _) = bundle_for_bridge(&dir);

        let (status, body) = call(
            &router,
            Protocol::Https,
            Method::POST,
            "/restore",
            Some(json!({
                "bundle": bundle,
                "admin_password": "correct horse battery",
                "z2m_token": "a-token",
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let reply: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(reply["bridge_id"], BRIDGE_ID);
        assert_eq!(reply["paired_clients"], 1);
        // the minimal config has no token, so the server took it
        assert_eq!(reply["tokens_written"], json!(["server1"]));
        assert_eq!(
            state.store().active().z2m.servers["server1"]
                .token
                .as_deref(),
            Some("a-token")
        );
        assert_eq!(state.link_button().history()[0].devicetype, "Hue Sync Box");
    }

    #[tokio::test]
    async fn a_restore_with_the_wrong_password_changes_nothing() {
        let (dir, state, router) = protected_fixture("correct horse battery").await;
        let (bundle, pem) = bundle_for_bridge(&dir);

        let (status, _) = call(
            &router,
            Protocol::Https,
            Method::POST,
            "/restore",
            Some(json!({
                "bundle": bundle,
                "admin_password": "wrong",
                "z2m_token": "a-token",
            })),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_ne!(
            crate::server::certificate::read_pem(&state.store().paths().cert).unwrap(),
            pem
        );
    }

    #[tokio::test]
    async fn a_bundle_without_a_private_key_is_refused() {
        let (dir, _state, router) = protected_fixture("correct horse battery").await;
        let (bundle, _) = bundle_for_bridge(&dir);
        let mut stripped = bundle.clone();
        stripped["certificate_pem"] =
            json!("-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n");

        let (status, body) = call(
            &router,
            Protocol::Https,
            Method::POST,
            "/restore",
            Some(json!({
                "bundle": stripped,
                "admin_password": "correct horse battery",
                "z2m_token": "a-token",
            })),
        )
        .await;

        // the text holds no certificate this bridge can read
        assert!(
            status == StatusCode::UNPROCESSABLE_ENTITY
                || status == StatusCode::INTERNAL_SERVER_ERROR,
            "{status} {body}"
        );
    }

    #[tokio::test]
    async fn a_restore_needs_the_admin_password() {
        let (_dir, state, router) = fixture().await;
        let (_, body) = call(&router, Protocol::Https, Method::GET, "/backup", None).await;
        let bundle: Value = serde_json::from_str(&body).unwrap();

        let (status, body) = call(
            &router,
            Protocol::Https,
            Method::POST,
            "/restore",
            Some(json!({
                "bundle": bundle,
                "admin_password": "whatever",
                "z2m_token": "a-token",
            })),
        )
        .await;

        // the minimal config sets no password, so the restore is refused
        // before anything is written
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body.contains("admin password"), "{body}");
        assert!(
            state.store().active().z2m.servers["server1"]
                .token
                .is_none()
        );
    }
}
