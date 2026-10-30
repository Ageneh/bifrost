//! Configuration validation.
//!
//! Hard errors stop a draft from being applied; warnings are only reported.
//! Startup never uses these rules as a gate, so a file that loads today keeps
//! loading.

use std::net::Ipv4Addr;
use std::str::FromStr;

use mac_address::MacAddress;
use serde_json::Value;
use url::Url;

pub use bifrost_api::admin::{Severity, ValidationError};

use crate::config::AppConfig;

const WEBSOCKET_SCHEMES: &[&str] = &["ws", "wss"];
const KNOWN_ROOM_KEYS: &[&str] = &["name", "icon"];
const IP_FIELDS: &[&str] = &["ipaddress", "netmask", "gateway"];
const PORT_FIELDS: &[&str] = &["http_port", "https_port", "entm_port"];

/// z2m 2.x only serves its websocket on this path, so a url ending in it is
/// written for 2.x.
const Z2M_V2_PATH_SUFFIX: &str = "/api";

/// Validate a parsed configuration.
#[must_use]
pub fn validate(config: &AppConfig) -> Vec<ValidationError> {
    let mut errors = vec![];

    check_port_collisions(config, &mut errors);
    check_z2m_servers(config, &mut errors);
    check_timezone(&config.bridge.timezone, &mut errors);

    // settings without a typed field are only visible in the document form
    if let Ok(document) = serde_json::to_value(config) {
        check_document(&document, &mut errors);
    }

    errors
}

/// Parse and validate a configuration supplied as a document, e.g. through
/// the admin API. Problems the parser would reject come back as hard errors
/// with the path of the offending setting.
pub fn validate_value(
    value: &Value,
) -> Result<(AppConfig, Vec<ValidationError>), Vec<ValidationError>> {
    let mut errors = vec![];
    check_formats(value, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }

    match serde_json::from_value::<AppConfig>(value.clone()) {
        Ok(config) => {
            let findings = validate(&config);
            Ok((config, findings))
        }
        Err(err) => Err(vec![ValidationError::error("", err.to_string())]),
    }
}

/// Hard errors in `draft` that `active` does not already have.
///
/// The running configuration started with its own findings, so those must
/// not block an unrelated change.
#[must_use]
pub fn new_errors(active: &AppConfig, draft: &AppConfig) -> Vec<ValidationError> {
    let existing = validate(active);
    validate(draft)
        .into_iter()
        .filter(|err| err.is_error() && !existing.contains(err))
        .collect()
}

fn check_port_collisions(config: &AppConfig, errors: &mut Vec<ValidationError>) {
    let bridge = &config.bridge;
    let ports = [
        ("bridge.http_port", bridge.http_port),
        ("bridge.https_port", bridge.https_port),
        ("bridge.entm_port", bridge.entm_port),
    ];

    for (index, (path, port)) in ports.iter().enumerate() {
        if let Some((other, _)) = ports[..index].iter().find(|(_, p)| p == port) {
            errors.push(ValidationError::error(
                *path,
                format!("port {port} is already used by {other}"),
            ));
        }
    }
}

fn check_z2m_servers(config: &AppConfig, errors: &mut Vec<ValidationError>) {
    if config.z2m.servers.is_empty() {
        errors.push(ValidationError::warning(
            "z2m",
            "no z2m servers configured; no lights will be available",
        ));
    }

    for (name, server) in &config.z2m.servers {
        // the backend for a server is the service `z2m@<name>`, so the name
        // has to survive that as one instance name
        if name.is_empty() || name.contains('@') {
            errors.push(ValidationError::error(
                format!("z2m.{name}"),
                "a z2m server name must not be empty or contain '@'",
            ));
        }

        let path = format!("z2m.{name}.url");
        let url = &server.url;

        if !WEBSOCKET_SCHEMES.contains(&url.scheme()) {
            errors.push(ValidationError::error(
                &path,
                format!("scheme must be ws or wss, not {}", url.scheme()),
            ));
        }

        let has_token = server.token.is_some() || url.query_pairs().any(|(key, _)| key == "token");
        if url.path().ends_with(Z2M_V2_PATH_SUFFIX) && !has_token {
            errors.push(ValidationError::warning(
                &path,
                "z2m 2.x requires a token; the z2m default token will be used",
            ));
        }
    }
}

fn check_timezone(timezone: &str, errors: &mut Vec<ValidationError>) {
    if tzfile::Tz::named(timezone).is_err() {
        errors.push(ValidationError::warning(
            "bridge.timezone",
            format!("timezone {timezone:?} is not known to this system"),
        ));
    }
}

fn check_document(document: &Value, errors: &mut Vec<ValidationError>) {
    let max_streams = document.pointer("/bifrost/entertainment/max_streams");
    if max_streams.and_then(Value::as_u64) == Some(0) {
        errors.push(ValidationError::error(
            "bifrost.entertainment.max_streams",
            "must be at least 1",
        ));
    }

    if let Some(rooms) = document.get("rooms").and_then(Value::as_object) {
        for (room, settings) in rooms {
            let Some(settings) = settings.as_object() else {
                continue;
            };
            for key in settings.keys() {
                if !KNOWN_ROOM_KEYS.contains(&key.as_str()) {
                    errors.push(ValidationError::warning(
                        format!("rooms.{room}.{key}"),
                        format!("unknown room setting; expected one of {KNOWN_ROOM_KEYS:?}"),
                    ));
                }
            }
        }
    }
}

/// Checks equivalent to what the parser rejects, but with a precise path.
fn check_formats(value: &Value, errors: &mut Vec<ValidationError>) {
    let bridge = value.get("bridge");

    check_string(bridge, "bridge", "mac", errors, |text| {
        MacAddress::from_str(text)
            .map(|_| ())
            .map_err(|_| "not a valid MAC address")
    });

    for field in IP_FIELDS {
        check_string(bridge, "bridge", field, errors, |text| {
            Ipv4Addr::from_str(text)
                .map(|_| ())
                .map_err(|_| "not a valid IPv4 address")
        });
    }

    for field in PORT_FIELDS {
        let Some(port) = bridge.and_then(|bridge| bridge.get(field)) else {
            continue;
        };
        if port
            .as_u64()
            .is_none_or(|port| u16::try_from(port).is_err())
        {
            errors.push(ValidationError::error(
                format!("bridge.{field}"),
                "must be a port number between 0 and 65535",
            ));
        }
    }

    if let Some(servers) = value.get("z2m").and_then(Value::as_object) {
        for (name, server) in servers {
            check_string(
                Some(server),
                &format!("z2m.{name}"),
                "url",
                errors,
                |text| Url::parse(text).map(|_| ()).map_err(|_| "not a valid url"),
            );
        }
    }
}

fn check_string(
    parent: Option<&Value>,
    parent_path: &str,
    field: &str,
    errors: &mut Vec<ValidationError>,
    check: impl Fn(&str) -> Result<(), &'static str>,
) {
    let Some(value) = parent.and_then(|parent| parent.get(field)) else {
        return;
    };

    let path = format!("{parent_path}.{field}");
    match value.as_str() {
        Some(text) => {
            if let Err(message) = check(text) {
                errors.push(ValidationError::error(path, message));
            }
        }
        None => errors.push(ValidationError::error(path, "must be a string")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::config::validate::{
        Severity, ValidationError, check_document, new_errors, validate, validate_value,
    };
    use crate::config::{self, AppConfig};
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    const NONE: Vec<ValidationError> = vec![];
    const NO_PATHS: Vec<&str> = vec![];

    fn parse(yaml: &str) -> AppConfig {
        let dir = TempDir::new();
        config::parse(&dir.write("config.yaml", yaml)).unwrap()
    }

    fn minimal() -> AppConfig {
        parse(MINIMAL_CONFIG)
    }

    fn minimal_value() -> Value {
        serde_json::to_value(minimal()).unwrap()
    }

    fn paths(errors: &[ValidationError], severity: Severity) -> Vec<&str> {
        errors
            .iter()
            .filter(|err| err.severity == severity)
            .map(|err| err.path.as_str())
            .collect()
    }

    fn findings_for(document: &Value) -> Vec<ValidationError> {
        let mut errors = vec![];
        check_document(document, &mut errors);
        errors
    }

    #[test]
    fn files_that_start_today_have_no_errors() {
        let example = include_str!("../../config.example.yaml");
        for yaml in [MINIMAL_CONFIG, example] {
            let errors = validate(&parse(yaml));
            assert_eq!(paths(&errors, Severity::Error), NO_PATHS, "{errors:?}");
        }
    }

    #[test]
    fn ports_distinct() {
        assert_eq!(paths(&validate(&minimal()), Severity::Error), NO_PATHS);
    }

    #[test]
    fn ports_http_https_collide() {
        let mut config = minimal();
        config.bridge.https_port = config.bridge.http_port;
        assert_eq!(
            paths(&validate(&config), Severity::Error),
            ["bridge.https_port"]
        );
    }

    #[test]
    fn ports_http_entm_collide() {
        let mut config = minimal();
        config.bridge.entm_port = config.bridge.http_port;
        assert_eq!(
            paths(&validate(&config), Severity::Error),
            ["bridge.entm_port"]
        );
    }

    #[test]
    fn ports_https_entm_collide() {
        let mut config = minimal();
        config.bridge.entm_port = config.bridge.https_port;
        assert_eq!(
            paths(&validate(&config), Severity::Error),
            ["bridge.entm_port"]
        );
    }

    #[test]
    fn ports_all_collide() {
        let mut config = minimal();
        config.bridge.https_port = 8080;
        config.bridge.http_port = 8080;
        config.bridge.entm_port = 8080;
        assert_eq!(
            paths(&validate(&config), Severity::Error),
            ["bridge.https_port", "bridge.entm_port"]
        );
    }

    #[test]
    fn url_scheme_ws_and_wss_accepted() {
        let mut config = minimal();
        let server = config.z2m.servers.get_mut("server1").unwrap();
        server.url = "wss://10.0.0.1:8080".parse().unwrap();
        assert_eq!(paths(&validate(&config), Severity::Error), NO_PATHS);
    }

    #[test]
    fn url_scheme_http_rejected() {
        let mut config = minimal();
        let server = config.z2m.servers.get_mut("server1").unwrap();
        server.url = "http://10.0.0.1:8080".parse().unwrap();
        assert_eq!(
            paths(&validate(&config), Severity::Error),
            ["z2m.server1.url"]
        );
    }

    #[test]
    fn max_streams_absent_accepted() {
        assert_eq!(findings_for(&json!({"bifrost": {}})), NONE);
    }

    #[test]
    fn max_streams_positive_accepted() {
        let document = json!({"bifrost": {"entertainment": {"max_streams": 1}}});
        assert_eq!(findings_for(&document), NONE);
    }

    #[test]
    fn max_streams_zero_rejected() {
        let document = json!({"bifrost": {"entertainment": {"max_streams": 0}}});
        assert_eq!(
            paths(&findings_for(&document), Severity::Error),
            ["bifrost.entertainment.max_streams"]
        );
    }

    #[test]
    fn z2m_v2_url_without_token_warns() {
        let mut config = minimal();
        let server = config.z2m.servers.get_mut("server1").unwrap();
        server.url = "ws://10.0.0.1:8080/api".parse().unwrap();
        assert_eq!(
            paths(&validate(&config), Severity::Warning),
            ["z2m.server1.url"]
        );
    }

    #[test]
    fn z2m_v2_url_with_token_accepted() {
        let mut config = minimal();
        let server = config.z2m.servers.get_mut("server1").unwrap();
        server.url = "ws://10.0.0.1:8080/api?token=abc".parse().unwrap();
        assert_eq!(validate(&config), NONE);
    }

    #[test]
    fn z2m_v2_url_with_token_field_accepted() {
        let mut config = minimal();
        let server = config.z2m.servers.get_mut("server1").unwrap();
        server.url = "ws://10.0.0.1:8080/api".parse().unwrap();
        server.token = Some("abc".into());
        assert_eq!(validate(&config), NONE);
    }

    #[test]
    fn unknown_room_key_in_parsed_config_warns() {
        let config = parse(&format!(
            "{MINIMAL_CONFIG}rooms:\n  office:\n    nmae: typo\n"
        ));
        assert_eq!(
            paths(&validate(&config), Severity::Warning),
            ["rooms.office.nmae"]
        );
    }

    #[test]
    fn max_streams_zero_in_parsed_config_rejected() {
        let config = parse(&format!(
            "{MINIMAL_CONFIG}bifrost:\n  entertainment:\n    max_streams: 0\n"
        ));
        assert_eq!(
            paths(&validate(&config), Severity::Error),
            ["bifrost.entertainment.max_streams"]
        );
    }

    #[test]
    fn z2m_v1_url_without_token_accepted() {
        assert_eq!(validate(&minimal()), NONE);
    }

    #[test]
    fn no_z2m_servers_warns() {
        let mut config = minimal();
        config.z2m.servers.clear();
        assert_eq!(paths(&validate(&config), Severity::Warning), ["z2m"]);
    }

    #[test]
    fn unknown_room_key_warns() {
        let document = json!({"rooms": {"office": {"name": "Office", "nmae": "typo"}}});
        assert_eq!(
            paths(&findings_for(&document), Severity::Warning),
            ["rooms.office.nmae"]
        );
    }

    #[test]
    fn known_room_keys_accepted() {
        let document = json!({"rooms": {"office": {"name": "Office", "icon": "office"}}});
        assert_eq!(findings_for(&document), NONE);
    }

    #[test]
    fn unknown_timezone_warns() {
        let mut config = minimal();
        config.bridge.timezone = "Mars/Olympus_Mons".into();
        assert_eq!(
            paths(&validate(&config), Severity::Warning),
            ["bridge.timezone"]
        );
    }

    #[test]
    fn known_timezone_accepted() {
        let mut config = minimal();
        config.bridge.timezone = "UTC".into();
        assert_eq!(validate(&config), NONE);
    }

    fn rejected_paths(value: &Value) -> Vec<String> {
        validate_value(value)
            .unwrap_err()
            .into_iter()
            .map(|err| {
                assert!(err.is_error());
                err.path
            })
            .collect()
    }

    #[test]
    fn value_valid_is_parsed() {
        let (config, findings) = validate_value(&minimal_value()).unwrap();
        assert_eq!(config.bridge.name, "Bifrost");
        assert_eq!(findings, NONE);
    }

    #[test]
    fn value_bad_mac_rejected() {
        let mut value = minimal_value();
        value["bridge"]["mac"] = json!("00:11:22");
        assert_eq!(rejected_paths(&value), ["bridge.mac"]);
    }

    #[test]
    fn value_bad_addresses_rejected() {
        for field in ["ipaddress", "netmask", "gateway"] {
            let mut value = minimal_value();
            value["bridge"][field] = json!("10.0.0.256");
            assert_eq!(rejected_paths(&value), [format!("bridge.{field}")]);
        }
    }

    #[test]
    fn value_bad_ports_rejected() {
        for port in [json!(65536), json!(-1), json!("80")] {
            let mut value = minimal_value();
            value["bridge"]["http_port"] = port;
            assert_eq!(rejected_paths(&value), ["bridge.http_port"]);
        }
    }

    #[test]
    fn value_bad_url_rejected() {
        let mut value = minimal_value();
        value["z2m"]["server1"]["url"] = json!("not a url");
        assert_eq!(rejected_paths(&value), ["z2m.server1.url"]);
    }

    #[test]
    fn value_missing_section_rejected() {
        let mut value = minimal_value();
        value.as_object_mut().unwrap().remove("bridge");
        assert_eq!(rejected_paths(&value), [""]);
    }

    #[test]
    fn existing_errors_do_not_block() {
        let mut active = minimal();
        active.bridge.entm_port = active.bridge.http_port;
        let mut draft = active.clone();
        draft.bridge.name = "Renamed".into();

        assert_eq!(new_errors(&active, &draft), NONE);

        draft.bridge.https_port = draft.bridge.http_port;
        assert_eq!(
            new_errors(&active, &draft)
                .iter()
                .map(|err| err.path.as_str())
                .collect::<Vec<_>>(),
            ["bridge.https_port"]
        );
    }
}
