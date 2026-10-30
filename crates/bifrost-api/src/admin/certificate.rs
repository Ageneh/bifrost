//! Wire types for the certificate panel under `/bifrost/bridge/certificate`.

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::error::BifrostResult;

/// What the bridge's own certificate says.
///
/// The panel is read-only. Regenerating invalidates every paired client, so
/// the endpoint exists but no screen calls it until pairing can be repeated.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CertificateInfo {
    /// Subject common name, which is the bridge id.
    pub common_name: String,
    /// The bridge id the certificate is issued for: the MAC address with the
    /// `ff:fe` middle bytes a Hue bridge id has.
    pub bridge_id: String,
    /// The MAC address the certificate derives from, as configured.
    pub mac: String,
    /// Start of validity, RFC 3339 in UTC.
    pub not_before: String,
    /// End of validity, RFC 3339 in UTC.
    pub not_after: String,
    /// Certificate serial number, hex.
    pub serial: String,
    /// SHA-256 over the DER encoding, uppercase hex with `:` between bytes.
    pub fingerprint: String,
}

impl Client {
    /// The bridge certificate, as the certificate panel shows it.
    pub async fn certificate(&self) -> BifrostResult<CertificateInfo> {
        self.get("bridge/certificate").await
    }

    /// Replace the bridge certificate with a freshly generated one, and
    /// return it. Invalidates every paired client.
    pub async fn regenerate_certificate(&self) -> BifrostResult<CertificateInfo> {
        self.post("bridge/certificate/regenerate", ()).await
    }
}
