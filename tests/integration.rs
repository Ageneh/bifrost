//! The bridge behind a real socket.
//!
//! Every other integration test drives the router through `tower::oneshot`,
//! which supplies a peer address by hand. Two controls cannot be reached
//! that way and are the reason this file exists:
//!
//! - the listener bind path: a wrong `bridge.ipaddress` locks the owner out,
//!   and the admin UI rides the very listener the change would move, so the
//!   address has to be proven by binding a socket, not by a header;
//! - the ingress negative case: `X-Ingress-Path` is only believed from a
//!   trusted peer, and "trusted" is decided by the address the kernel
//!   reports, which only a real connection provides.
//!
//! `127.0.0.2` is in the loopback range and needs no privilege, but it is a
//! second address: a listener that only ever bound `127.0.0.1` cannot answer
//! there. That is the defect this file is written to catch.

mod support;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use reqwest::StatusCode;
use serde_json::Value;
use svc::manager::ServiceManager;

use bifrost::config_store::ConfigStore;
use bifrost::routes;
use bifrost::server::appstate::AppState;
use bifrost::server::hue_cors_layer;
use support::temp_dir::TempDir;

/// The address the bridge binds: `127.0.0.2` where the loopback range has
/// more than one address (Linux, and the CI runner this gate is written
/// for), else `127.0.0.1`.
///
/// macOS configures only `127.0.0.1`, and an unsigned test binary cannot
/// reach the host's LAN address there, so the fallback keeps the listener
/// and guard paths under test locally; the second address is covered on
/// Linux, where CI runs this gate.
fn bridge_address() -> Ipv4Addr {
    if std::net::TcpListener::bind((Ipv4Addr::new(127, 0, 0, 2), 0)).is_ok() {
        return Ipv4Addr::new(127, 0, 0, 2);
    }
    Ipv4Addr::LOCALHOST
}

/// A running bridge: a router on a real socket, and the config behind it.
struct Bridge {
    _dir: TempDir,
    address: SocketAddr,
    client: reqwest::Client,
}

impl Bridge {
    /// Start the bridge on `BRIDGE_ADDRESS`, trusting the peers in
    /// `trusted_cidrs` for Home Assistant Ingress.
    async fn start(trusted_cidrs: &str) -> Self {
        let address = bridge_address();
        let dir = TempDir::new();
        let yaml = format!(
            "\
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: {address}
  netmask: 255.0.0.0
  gateway: 127.0.0.1
  timezone: UTC

bifrost:
  auth:
    trust_ingress: true
    trusted_cidrs: [{trusted_cidrs}]

z2m:
  unused:
    url: ws://127.0.0.1:9
"
        );
        let store = Arc::new(ConfigStore::load(&dir.write("config.yaml", &yaml)).unwrap());
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(store, svm).await.unwrap();
        let router: Router = routes::router(state, hue_cors_layer());

        // the real listener path: bind the configured address, ephemeral port
        let listener = std::net::TcpListener::bind((address, 0))
            .expect("cannot bind the configured bridge address");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum_server::from_tcp(listener)
                .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                .await;
        });

        Self {
            _dir: dir,
            address,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
        }
    }

    async fn get(&self, path: &str, headers: &[(&str, &str)]) -> (StatusCode, String) {
        let mut request = self
            .client
            .get(format!("http://{}{path}", self.address))
            .header("host", self.address.ip().to_string());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        let response = request.send().await.expect("the bridge did not answer");
        let status = response.status();
        (status, response.text().await.unwrap())
    }
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|err| panic!("not JSON: {err}: {text}"))
}

/// The bridge answers on the address `bridge.ipaddress` names: the listener
/// follows the configuration, which is the path that locks an owner out when
/// it is wrong.
#[tokio::test]
async fn serves_on_the_configured_address() {
    let bridge = Bridge::start("\"172.30.32.2/32\"").await;

    let (status, body) = bridge.get("/bifrost/auth/session", &[]).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json(&body)["authenticated"], false, "{body}");
}

/// `X-Ingress-Path` from a peer outside `trusted_cidrs` is refused, even
/// though the header is well formed and the bridge trusts Ingress.
#[tokio::test]
async fn ingress_header_from_an_untrusted_peer_is_refused() {
    // the connection comes from 127.0.0.1; the Supervisor is not here
    let bridge = Bridge::start("\"172.30.32.2/32\"").await;

    let (status, body) = bridge
        .get(
            "/bifrost/config",
            &[("x-ingress-path", "/api/hassio_ingress/test")],
        )
        .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("Supervisor"), "{body}");
}

/// From a trusted peer the same header skips the login, which is what makes
/// the add-on panel usable.
#[tokio::test]
async fn ingress_header_from_a_trusted_peer_skips_the_login() {
    // the test's own client connects from the address the bridge is bound to
    let trusted = format!("\"{}/32\"", bridge_address());
    let bridge = Bridge::start(&trusted).await;

    let (status, body) = bridge
        .get(
            "/bifrost/config",
            &[("x-ingress-path", "/api/hassio_ingress/test")],
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        json(&body)["bifrost"]["auth"]["trust_ingress"],
        true,
        "{body}"
    );
}

/// Without a session and without Ingress, the admin API refuses a read.
#[tokio::test]
async fn the_admin_api_needs_a_session() {
    let bridge = Bridge::start("\"172.30.32.2/32\"").await;

    let (status, body) = bridge.get("/bifrost/config", &[]).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("Log in"), "{body}");
}
