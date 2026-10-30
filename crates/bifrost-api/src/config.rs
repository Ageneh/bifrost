use std::fmt::{self, Debug, Display};
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::{collections::BTreeMap, num::NonZeroU32};

use camino::Utf8PathBuf;
use hue::api::RoomArchetype;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
pub const DEFAULT_SESSION_HOURS: u32 = 12;
pub const DEFAULT_IDLE_MINUTES: u32 = 60;

/// The address the Home Assistant Supervisor's Ingress proxy connects from.
/// Home Assistant's add-on documentation tells add-ons to accept Ingress
/// connections from this address only.
pub const SUPERVISOR_INGRESS_PEER: Ipv4Addr = Ipv4Addr::new(172, 30, 32, 2);

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

const fn default_session_hours() -> u32 {
    DEFAULT_SESSION_HOURS
}

const fn default_idle_minutes() -> u32 {
    DEFAULT_IDLE_MINUTES
}

const fn default_trust_ingress() -> bool {
    true
}

fn default_trusted_cidrs() -> Vec<IpCidr> {
    vec![IpCidr::host(IpAddr::V4(SUPERVISOR_INGRESS_PEER))]
}

#[derive(Clone, Serialize, Deserialize)]
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

#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct BifrostConfig {
    #[serde(default = "default_state_file")]
    pub state_file: Utf8PathBuf,
    #[serde(default = "default_cert_file")]
    pub cert_file: Utf8PathBuf,
    /// Admin authentication. Never changed through the configuration draft:
    /// only the `/bifrost/auth` endpoints, which require the password, write
    /// it.
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(flatten)]
    pub unknown: UnknownSettings,
}

impl Default for BifrostConfig {
    fn default() -> Self {
        Self {
            state_file: default_state_file(),
            cert_file: default_cert_file(),
            auth: AuthConfig::default(),
            unknown: UnknownSettings::new(),
        }
    }
}

/// An IP network, written `10.0.0.0/8`; a bare address is a single host.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IpCidr {
    addr: IpAddr,
    prefix: u8,
}

impl IpCidr {
    #[must_use]
    pub const fn host(addr: IpAddr) -> Self {
        let prefix = if addr.is_ipv4() { 32 } else { 128 };
        Self { addr, prefix }
    }

    /// Whether `addr` lies inside this network. IPv4 and IPv6 never match
    /// each other, except an IPv4-mapped IPv6 address.
    #[must_use]
    pub fn contains(&self, addr: IpAddr) -> bool {
        let addr = match addr {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(addr, IpAddr::V4),
            IpAddr::V4(_) => addr,
        };
        match (self.addr, addr) {
            (IpAddr::V4(net), IpAddr::V4(addr)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(addr) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(addr)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(addr) & mask
            }
            _ => false,
        }
    }
}

impl FromStr for IpCidr {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (addr, prefix) = text
            .split_once('/')
            .map_or((text, None), |(addr, prefix)| (addr, Some(prefix)));
        let addr: IpAddr = addr
            .trim()
            .parse()
            .map_err(|_| format!("{text:?} is not an IP address or network"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(prefix) => prefix
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|prefix| *prefix <= max)
                .ok_or_else(|| format!("{text:?} has an invalid prefix length"))?,
        };
        Ok(Self { addr, prefix })
    }
}

impl Display for IpCidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl Debug for IpCidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Serialize for IpCidr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for IpCidr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Admin authentication for `/bifrost`. The Hue API stays open.
#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct AuthConfig {
    /// Hash of the admin password. Without one, Bifrost prints a setup code
    /// at startup and waits for the first password. Deleting it is how a
    /// lost password is reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<String>,
    /// A session ends this many hours after login, however active.
    #[serde(default = "default_session_hours")]
    pub session_hours: u32,
    /// A session ends after this many minutes without a request.
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u32,
    /// Let Home Assistant Ingress requests from `trusted_cidrs` in without a
    /// Bifrost login: Home Assistant authenticated the user already.
    #[serde(default = "default_trust_ingress")]
    pub trust_ingress: bool,
    /// Peers whose Ingress requests are trusted. Decided by the connecting
    /// address only, never by a header.
    #[serde(default = "default_trusted_cidrs")]
    pub trusted_cidrs: Vec<IpCidr>,
    /// Extra host names the admin interface answers to, besides the bridge
    /// address and its `.local` name; e.g. a DNS name pointing at Bifrost.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(flatten)]
    pub unknown: UnknownSettings,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            password_hash: None,
            session_hours: default_session_hours(),
            idle_minutes: default_idle_minutes(),
            trust_ingress: default_trust_ingress(),
            trusted_cidrs: default_trusted_cidrs(),
            allowed_hosts: vec![],
            unknown: UnknownSettings::new(),
        }
    }
}

impl Debug for AuthConfig {
    // the password hash must never reach a log line
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field(
                "password_hash",
                &self.password_hash.as_ref().map(|_| REDACTED_PLACEHOLDER),
            )
            .field("session_hours", &self.session_hours)
            .field("idle_minutes", &self.idle_minutes)
            .field("trust_ingress", &self.trust_ingress)
            .field("trusted_cidrs", &self.trusted_cidrs)
            .field("allowed_hosts", &self.allowed_hosts)
            .field("unknown", &SettingNames(&self.unknown))
            .finish()
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

#[derive(Clone, Serialize, Deserialize, Default, Eq, PartialEq)]
pub struct RoomConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<RoomArchetype>,
    #[serde(flatten)]
    pub unknown: UnknownSettings,
}

#[derive(Clone, Serialize, Deserialize)]
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

/// What a secret reads as wherever it leaves the process.
const REDACTED_PLACEHOLDER: &str = "<<REDACTED>>";

/// `Debug` for settings this version does not know: their names only, since
/// any of their values could be a secret.
struct SettingNames<'a>(&'a UnknownSettings);

impl Debug for SettingNames<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

// Debug output ends up in logs, so every settings struct prints unknown
// settings by name only, and secrets not at all

impl Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("bridge", &self.bridge)
            .field("z2m", &self.z2m)
            .field("bifrost", &self.bifrost)
            .field("rooms", &self.rooms)
            .field("unknown", &SettingNames(&self.unknown))
            .finish()
    }
}

impl Debug for BridgeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeConfig")
            .field("name", &self.name)
            .field("mac", &self.mac)
            .field("ipaddress", &self.ipaddress)
            .field("http_port", &self.http_port)
            .field("https_port", &self.https_port)
            .field("entm_port", &self.entm_port)
            .field("netmask", &self.netmask)
            .field("gateway", &self.gateway)
            .field("timezone", &self.timezone)
            .field("unknown", &SettingNames(&self.unknown))
            .finish()
    }
}

impl Debug for BifrostConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BifrostConfig")
            .field("state_file", &self.state_file)
            .field("cert_file", &self.cert_file)
            .field("auth", &self.auth)
            .field("unknown", &SettingNames(&self.unknown))
            .finish()
    }
}

impl Debug for RoomConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoomConfig")
            .field("name", &self.name)
            .field("icon", &self.icon)
            .field("unknown", &SettingNames(&self.unknown))
            .finish()
    }
}

impl Debug for Z2mServer {
    // urls and tokens can carry secrets, and Debug output ends up in logs
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Z2mServer")
            .field("url", &Self::sanitize_url(self.url.as_str()))
            .field("group_prefix", &self.group_prefix)
            .field("disable_tls_verify", &self.disable_tls_verify)
            .field("streaming_fps", &self.streaming_fps)
            .field("token", &self.token.as_ref().map(|_| REDACTED_PLACEHOLDER))
            .field("unknown", &SettingNames(&self.unknown))
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

    /// Replace a non-default `token=` value and any password in `url` with
    /// a placeholder. A user name without a password is replaced too: it
    /// may itself be a token.
    #[must_use]
    #[allow(clippy::option_if_let_else)]
    pub fn sanitize_url(url: &str) -> String {
        let url = Self::sanitize_userinfo(url);
        match url.find("token=") {
            Some(offset) => {
                let token = &url[offset + "token=".len()..];
                if token == DEFAULT_Z2M_TOKEN {
                    // this is the standard "blank" token, it's safe to show
                    url
                } else {
                    // this is an actual secret token, blank it out with a
                    // standard-length placeholder.
                    format!("{}token={}", &url[..offset], REDACTED_PLACEHOLDER)
                }
            }
            None => url,
        }
    }

    /// Redact the `user:password@` part of `url`, if it has one.
    fn sanitize_userinfo(url: &str) -> String {
        let Some(scheme_end) = url.find("://").map(|offset| offset + "://".len()) else {
            return url.to_string();
        };
        let rest = &url[scheme_end..];
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let Some(at) = rest[..authority_end].rfind('@') else {
            return url.to_string();
        };

        let userinfo = match rest[..at].split_once(':') {
            Some((user, _password)) => format!("{user}:{REDACTED_PLACEHOLDER}"),
            None => REDACTED_PLACEHOLDER.to_string(),
        };
        format!("{}{userinfo}{}", &url[..scheme_end], &rest[at..])
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

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{AppConfig, AuthConfig, IpCidr, Z2mServer};

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn cidr_parses_networks_and_hosts() {
        let net: IpCidr = "10.0.0.0/8".parse().unwrap();
        assert!(net.contains(ip("10.255.0.1")));
        assert!(!net.contains(ip("11.0.0.1")));

        let host: IpCidr = "172.30.32.2".parse().unwrap();
        assert_eq!(host.to_string(), "172.30.32.2/32");
        assert!(host.contains(ip("172.30.32.2")));
        assert!(host.contains(ip("::ffff:172.30.32.2")));
        assert!(!host.contains(ip("172.30.32.5")));

        let any: IpCidr = "0.0.0.0/0".parse().unwrap();
        assert!(any.contains(ip("8.8.8.8")));
        assert!(!any.contains(ip("fe80::1")));

        let v6: IpCidr = "fd00::/8".parse().unwrap();
        assert!(v6.contains(ip("fd12::1")));
        assert!(!v6.contains(ip("fe80::1")));
    }

    #[test]
    fn cidr_rejects_garbage() {
        for bad in [
            "",
            "10.0.0.0/33",
            "fd00::/129",
            "10.0.0/8",
            "host.local",
            "10.0.0.0/x",
        ] {
            assert!(bad.parse::<IpCidr>().is_err(), "{bad}");
        }
    }

    #[test]
    fn auth_defaults_trust_only_the_supervisor() {
        let auth: AuthConfig = serde_json::from_str("{}").unwrap();

        assert_eq!(auth.trusted_cidrs, ["172.30.32.2/32".parse().unwrap()]);
        assert!(auth.trust_ingress);
        assert_eq!(auth.password_hash, None);
        assert_eq!(
            serde_json::to_value(&auth).unwrap()["trusted_cidrs"],
            serde_json::json!(["172.30.32.2/32"])
        );
    }

    #[test]
    fn sanitize_url_redacts_userinfo_and_tokens() {
        for (url, expected) in [
            (
                "ws://user:hunter2@10.0.0.1:8080/api",
                "ws://user:<<REDACTED>>@10.0.0.1:8080/api",
            ),
            (
                "wss://hunter2@10.0.0.1/api?token=hunter3",
                "wss://<<REDACTED>>@10.0.0.1/api?token=<<REDACTED>>",
            ),
            (
                "ws://10.0.0.1/api?token=hunter3",
                "ws://10.0.0.1/api?token=<<REDACTED>>",
            ),
            (
                "ws://10.0.0.1/api?token=your-secret-token",
                "ws://10.0.0.1/api?token=your-secret-token",
            ),
            // an @ after the authority is not userinfo
            ("ws://10.0.0.1/a@b", "ws://10.0.0.1/a@b"),
            ("not a url", "not a url"),
        ] {
            assert_eq!(Z2mServer::sanitize_url(url), expected, "{url}");
        }
    }

    #[test]
    fn debug_shows_unknown_settings_by_name_only() {
        let config: AppConfig = serde_json::from_value(serde_json::json!({
            "bridge": {
                "name": "Bifrost", "mac": "00:11:22:33:44:55", "ipaddress": "10.0.0.2",
                "netmask": "255.255.255.0", "gateway": "10.0.0.1", "timezone": "UTC",
                "api_secret": "hunter-bridge"
            },
            "z2m": {"s1": {"url": "ws://u:hunter-url@10.0.0.3/api", "extra": "hunter-server"}},
            "bifrost": {"auth": {"password_hash": "hunter-hash"}, "x": "hunter-bifrost"},
            "rooms": {"office": {"secret_note": "hunter-room"}},
            "future": {"password": "hunter-top"}
        }))
        .unwrap();

        let debug = format!("{config:?}");

        assert!(!debug.contains("hunter"), "{debug}");
        for name in ["api_secret", "extra", "secret_note", "future"] {
            assert!(debug.contains(name), "{name} missing: {debug}");
        }
    }

    #[test]
    fn auth_debug_hides_the_hash() {
        let auth = AuthConfig {
            password_hash: Some("$scrypt$secret-hash".into()),
            ..AuthConfig::default()
        };
        let debug = format!("{auth:?}");
        assert!(!debug.contains("secret-hash"), "{debug}");
    }
}
