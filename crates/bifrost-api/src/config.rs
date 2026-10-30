use std::fmt::{self, Debug};
use std::net::Ipv4Addr;
use std::{collections::BTreeMap, num::NonZeroU32};

use camino::Utf8PathBuf;
use hue::api::RoomArchetype;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::{Client, error::BifrostResult};

#[cfg(feature = "mac")]
use mac_address::MacAddress;
#[cfg(not(feature = "mac"))]
type MacAddress = String;

/// Settings this version does not know, kept so that saving a file never
/// drops them.
pub type UnknownSettings = BTreeMap<String, Value>;

pub const DEFAULT_HTTP_PORT: u16 = 80;
pub const DEFAULT_HTTPS_PORT: u16 = 443;
pub const DEFAULT_ENTM_PORT: u16 = 2100;
pub const DEFAULT_STATE_FILE: &str = "state.yaml";
pub const DEFAULT_CERT_FILE: &str = "cert.pem";

const fn default_http_port() -> u16 {
    DEFAULT_HTTP_PORT
}

const fn default_https_port() -> u16 {
    DEFAULT_HTTPS_PORT
}

const fn default_entm_port() -> u16 {
    DEFAULT_ENTM_PORT
}

fn default_state_file() -> Utf8PathBuf {
    DEFAULT_STATE_FILE.into()
}

fn default_cert_file() -> Utf8PathBuf {
    DEFAULT_CERT_FILE.into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub name: String,
    pub mac: MacAddress,
    pub ipaddress: Ipv4Addr,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_https_port")]
    pub https_port: u16,
    #[serde(default = "default_entm_port")]
    pub entm_port: u16,
    pub netmask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub timezone: String,
    #[serde(flatten)]
    pub unknown: UnknownSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct BifrostConfig {
    #[serde(default = "default_state_file")]
    pub state_file: Utf8PathBuf,
    #[serde(default = "default_cert_file")]
    pub cert_file: Utf8PathBuf,
    #[serde(flatten)]
    pub unknown: UnknownSettings,
}

impl Default for BifrostConfig {
    fn default() -> Self {
        Self {
            state_file: default_state_file(),
            cert_file: default_cert_file(),
            unknown: UnknownSettings::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Z2mConfig {
    #[serde(flatten)]
    pub servers: BTreeMap<String, Z2mServer>,
}

#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct Z2mServer {
    pub url: Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_tls_verify: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_fps: Option<NonZeroU32>,
    /// z2m 2.x websocket auth token. Takes precedence over a `token` in the
    /// url query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(flatten)]
    pub unknown: UnknownSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, Eq, PartialEq)]
pub struct RoomConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<RoomArchetype>,
    #[serde(flatten)]
    pub unknown: UnknownSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub bridge: BridgeConfig,
    pub z2m: Z2mConfig,
    #[serde(default)]
    pub bifrost: BifrostConfig,
    #[serde(default)]
    pub rooms: BTreeMap<String, RoomConfig>,
    #[serde(flatten)]
    pub unknown: UnknownSettings,
}

/// The url query parameter z2m 2.x reads its auth token from.
const TOKEN_QUERY_KEY: &str = "token";

/// The token z2m 2.x uses when none is configured.
const DEFAULT_Z2M_TOKEN: &str = "your-secret-token";

impl Debug for Z2mServer {
    // urls and tokens can carry secrets, and Debug output ends up in logs
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Z2mServer")
            .field("url", &Self::sanitize_url(self.url.as_str()))
            .field("group_prefix", &self.group_prefix)
            .field("disable_tls_verify", &self.disable_tls_verify)
            .field("streaming_fps", &self.streaming_fps)
            .field("token", &self.token.as_ref().map(|_| "<<REDACTED>>"))
            .field("unknown", &self.unknown.keys())
            .finish()
    }
}

impl AppConfig {
    /// Move every z2m token given in a url query into the `token` field.
    /// The url `get_url()` builds stays the same. Returns whether anything
    /// changed.
    pub fn migrate_url_tokens(&mut self) -> bool {
        let mut changed = false;
        for server in self.z2m.servers.values_mut() {
            changed |= server.migrate_url_token();
        }
        changed
    }
}

impl Z2mServer {
    /// Move a `token` from the url query into the `token` field, unless the
    /// field is already set.
    pub fn migrate_url_token(&mut self) -> bool {
        if self.token.is_some() {
            return false;
        }

        let Some(token) = self
            .url
            .query_pairs()
            .find(|(key, _)| key == TOKEN_QUERY_KEY)
            .map(|(_, value)| value.into_owned())
        else {
            return false;
        };

        let rest: Vec<(String, String)> = self
            .url
            .query_pairs()
            .filter(|(key, _)| key != TOKEN_QUERY_KEY)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();

        if rest.is_empty() {
            self.url.set_query(None);
        } else {
            self.url.query_pairs_mut().clear().extend_pairs(rest);
        }
        self.token = Some(token);

        true
    }

    #[must_use]
    pub fn get_url(&self) -> Url {
        let mut url = self.url.clone();
        // z2m version 1.x allows both / and /api as endpoints for the
        // websocket, but version 2.x only allows /api. By adding /api (if
        // missing), we ensure compatibility with both versions.
        if !url.path().ends_with("/api") {
            if let Ok(mut path) = url.path_segments_mut() {
                path.push("api");
            }
        }

        // z2m version 2.x requires an auth token on the websocket. If one is
        // not specified in the z2m configuration, the literal string
        // `your-secret-token` is used!
        //
        // To be compatible, we mirror this behavior here. If "token" is set
        // manually by the user, we do nothing.
        //
        // A token in the `token` field replaces any token in the url.
        if let Some(token) = &self.token {
            let rest: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(key, _)| key != TOKEN_QUERY_KEY)
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(rest)
                .append_pair(TOKEN_QUERY_KEY, token);
        } else if !url.query_pairs().any(|(key, _)| key == TOKEN_QUERY_KEY) {
            url.query_pairs_mut()
                .append_pair(TOKEN_QUERY_KEY, DEFAULT_Z2M_TOKEN);
        }

        url
    }

    /// Replace a non-default `token=` value in `url` with a placeholder.
    #[must_use]
    #[allow(clippy::option_if_let_else)]
    pub fn sanitize_url(url: &str) -> String {
        match url.find("token=") {
            Some(offset) => {
                let token = &url[offset + "token=".len()..];
                if token == DEFAULT_Z2M_TOKEN {
                    // this is the standard "blank" token, it's safe to show
                    url.to_string()
                } else {
                    // this is an actual secret token, blank it out with a
                    // standard-length placeholder.
                    format!("{}token={}", &url[..offset], "<<REDACTED>>")
                }
            }
            None => url.to_string(),
        }
    }

    #[must_use]
    pub fn get_sanitized_url(&self) -> String {
        Self::sanitize_url(self.get_url().as_str())
    }
}

impl Client {
    pub async fn config(&self) -> BifrostResult<AppConfig> {
        self.get("config").await
    }
}
