//! Wire types for z2m server administration under `/bifrost/z2m`.
//!
//! Every write goes through the configuration draft, so nothing changes
//! until the draft is applied, and a stale revision is refused with
//! `409 Conflict`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use svc::traits::ServiceState;

use crate::Client;
use crate::admin::DraftResponse;
use crate::config::Z2mServer;
use crate::error::BifrostResult;

/// One configured z2m server, with the state of the backend serving it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Z2mServerEntry {
    /// Name the server is configured under; its backend is `z2m@<name>`.
    pub name: String,
    /// State of the backend, `null` when no backend is registered for it yet.
    pub state: Option<ServiceState>,
    /// The server's settings, with secrets redacted.
    pub config: Value,
}

/// Every configured z2m server.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Z2mServerList {
    pub servers: Vec<Z2mServerEntry>,
}

/// Create or replace the z2m server `name` in the draft.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Z2mServerWrite {
    pub revision: u64,
    pub server: Z2mServer,
}

impl Client {
    /// Every configured z2m server, with the state of its backend.
    pub async fn z2m_servers(&self) -> BifrostResult<Z2mServerList> {
        self.get("z2m").await
    }

    /// Create or replace a z2m server in the draft.
    pub async fn put_z2m_server(
        &self,
        name: &str,
        revision: u64,
        server: Z2mServer,
    ) -> BifrostResult<DraftResponse> {
        self.put(&format!("z2m/{name}"), Z2mServerWrite { revision, server })
            .await
    }

    /// Remove a z2m server from the draft.
    pub async fn delete_z2m_server(
        &self,
        name: &str,
        revision: u64,
    ) -> BifrostResult<DraftResponse> {
        self.delete(
            &format!("z2m/{name}"),
            crate::admin::RevisionRequest { revision },
        )
        .await
    }
}
