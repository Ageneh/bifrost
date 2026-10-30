//! Differences between two configurations, and what applying them takes.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

pub use bifrost_api::admin::{ApplyAction, ApplyPlan, ConfigChange, Reason};

use crate::config::AppConfig;
use crate::config::redact::redacted;

pub const HTTP_SERVICE: &str = "http";
pub const HTTPS_SERVICE: &str = "https";
pub const ENTERTAINMENT_SERVICE: &str = "entertainment";
pub const MDNS_SERVICE: &str = "mdns";
pub const SSDP_SERVICE: &str = "ssdp";

/// Services bound to the bridge address, and therefore restarted when the
/// address changes.
pub const NETWORK_SERVICES: &[&str] = &[
    HTTP_SERVICE,
    HTTPS_SERVICE,
    ENTERTAINMENT_SERVICE,
    MDNS_SERVICE,
    SSDP_SERVICE,
];

/// A changed leaf, before redaction.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    path: Vec<String>,
    before: Value,
    after: Value,
}

/// Every setting that differs between `old` and `new`, with secrets
/// redacted.
#[must_use]
pub fn diff(old: &AppConfig, new: &AppConfig) -> Vec<ConfigChange> {
    changes(old, new).iter().map(redact_change).collect()
}

/// Classify the changes between `old` and `new` into what can be applied
/// while running and what needs a restart.
#[must_use]
pub fn plan(old: &AppConfig, new: &AppConfig) -> ApplyPlan {
    plan_changes(changes(old, new))
}

fn plan_changes(changes: Vec<Change>) -> ApplyPlan {
    let mut plan = ApplyPlan::default();

    for change in changes {
        let path = change.path.join(".");
        match classify(&change) {
            Effect::Live => push_unique(&mut plan.hot, ApplyAction::Update { path }),
            Effect::Actions(actions) => {
                for action in actions {
                    push_unique(&mut plan.hot, action);
                }
            }
            Effect::Restart(message) => plan.restart_required.push(Reason {
                path,
                message: message.to_string(),
            }),
        }
    }

    plan
}

enum Effect {
    Live,
    Actions(Vec<ApplyAction>),
    Restart(&'static str),
}

fn restart_services(services: &[&str]) -> Effect {
    Effect::Actions(
        services
            .iter()
            .map(|service| ApplyAction::RestartService {
                service: (*service).to_string(),
            })
            .collect(),
    )
}

fn classify(change: &Change) -> Effect {
    let path: Vec<&str> = change.path.iter().map(String::as_str).collect();

    match path.as_slice() {
        ["bridge", "http_port"] => restart_services(&[HTTP_SERVICE]),
        ["bridge", "https_port"] => restart_services(&[HTTPS_SERVICE]),
        ["bridge", "entm_port"] => restart_services(&[ENTERTAINMENT_SERVICE]),
        ["bridge", "ipaddress" | "netmask" | "gateway"] => restart_services(NETWORK_SERVICES),
        ["bridge", "mac"] => Effect::Restart(
            "the bridge identity (certificate, bridge id) is derived from the MAC address at startup",
        ),
        ["bifrost", "state_file"] => Effect::Restart("the state file is only read at startup"),
        ["bifrost", "cert_file"] => Effect::Restart("the certificate is only loaded at startup"),
        ["z2m", name] => {
            let name = (*name).to_string();
            Effect::Actions(vec![if change.before.is_null() {
                ApplyAction::StartBackend { name }
            } else if change.after.is_null() {
                ApplyAction::StopBackend { name }
            } else {
                ApplyAction::RestartBackend { name }
            }])
        }
        ["z2m", name, ..] => Effect::Actions(vec![ApplyAction::RestartBackend {
            name: (*name).to_string(),
        }]),
        _ => Effect::Live,
    }
}

fn push_unique(actions: &mut Vec<ApplyAction>, action: ApplyAction) {
    if !actions.contains(&action) {
        actions.push(action);
    }
}

fn changes(old: &AppConfig, new: &AppConfig) -> Vec<Change> {
    let (Ok(old), Ok(new)) = (serde_json::to_value(old), serde_json::to_value(new)) else {
        // AppConfig only contains types that always serialize
        return vec![];
    };

    let mut out = vec![];
    diff_values(&mut vec![], &old, &new, &mut out);
    out
}

/// Recursive structural diff. Objects are compared key by key (the same
/// shape as `hue::diff`, minus its event whitelist); anything else is
/// compared as a whole.
fn diff_values(path: &mut Vec<String>, a: &Value, b: &Value, out: &mut Vec<Change>) {
    if let (Value::Object(a), Value::Object(b)) = (a, b) {
        let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        for key in keys {
            path.push(key.clone());
            diff_values(
                path,
                a.get(key).unwrap_or(&Value::Null),
                b.get(key).unwrap_or(&Value::Null),
                out,
            );
            path.pop();
        }
    } else if a != b {
        out.push(Change {
            path: path.clone(),
            before: a.clone(),
            after: b.clone(),
        });
    }
}

/// Redact a changed value as if it still sat under its key, so leaf secrets,
/// urls and nested secrets all follow the same rules.
fn redact_leaf(key: &str, value: &Value) -> Value {
    let mut wrapper = Map::new();
    wrapper.insert(key.to_string(), value.clone());
    let mut wrapper = redacted(&Value::Object(wrapper));
    wrapper[key].take()
}

fn redact_change(change: &Change) -> ConfigChange {
    let key = change.path.last().map_or("", String::as_str);
    let before = redact_leaf(key, &change.before);
    let after = redact_leaf(key, &change.after);
    let secret = before != change.before || after != change.after;

    ConfigChange {
        path: change.path.join("."),
        before,
        after,
        secret,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::config::diff::{
        ApplyAction, ApplyPlan, Change, ENTERTAINMENT_SERVICE, HTTP_SERVICE, HTTPS_SERVICE,
        NETWORK_SERVICES, diff, plan, plan_changes,
    };
    use crate::config::redact::REDACTED;
    use crate::config::{self, AppConfig};
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    fn minimal() -> AppConfig {
        let dir = TempDir::new();
        config::parse(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap()
    }

    fn change(path: &str, before: Value, after: Value) -> Change {
        Change {
            path: path.split('.').map(str::to_string).collect(),
            before,
            after,
        }
    }

    fn update(path: &str) -> ApplyAction {
        ApplyAction::Update {
            path: path.to_string(),
        }
    }

    fn restart_service(service: &str) -> ApplyAction {
        ApplyAction::RestartService {
            service: service.to_string(),
        }
    }

    fn assert_live(path: &str) {
        let plan = plan_changes(vec![change(path, json!(1), json!(2))]);
        assert_eq!(plan.hot, [update(path)], "{path}");
        assert_eq!(plan.restart_required, [], "{path}");
    }

    fn assert_restart_required(path: &str) {
        let plan = plan_changes(vec![change(path, json!("a"), json!("b"))]);
        assert_eq!(plan.hot, [], "{path}");
        assert_eq!(plan.restart_required.len(), 1, "{path}");
        assert_eq!(plan.restart_required[0].path, path);
    }

    #[test]
    fn identical_configs_have_no_changes() {
        let config = minimal();
        assert_eq!(diff(&config, &config), []);
        assert_eq!(plan(&config, &config), ApplyPlan::default());
    }

    #[test]
    fn live_settings_apply_hot() {
        for path in [
            "bridge.name",
            "bridge.timezone",
            "rooms.office.name",
            "devices.lamp.name",
            "auth.users",
            "pairing.enabled",
            "bifrost.entertainment.max_streams",
        ] {
            assert_live(path);
        }
    }

    #[test]
    fn port_change_restarts_its_listener() {
        for (path, service) in [
            ("bridge.http_port", HTTP_SERVICE),
            ("bridge.https_port", HTTPS_SERVICE),
            ("bridge.entm_port", ENTERTAINMENT_SERVICE),
        ] {
            let plan = plan_changes(vec![change(path, json!(80), json!(8080))]);
            assert_eq!(plan.hot, [restart_service(service)], "{path}");
            assert_eq!(plan.restart_required, []);
        }
    }

    #[test]
    fn address_change_restarts_all_network_services() {
        let expected: Vec<ApplyAction> = NETWORK_SERVICES
            .iter()
            .map(|service| restart_service(service))
            .collect();

        for path in ["bridge.ipaddress", "bridge.netmask", "bridge.gateway"] {
            let plan = plan_changes(vec![change(path, json!("10.0.0.1"), json!("10.0.0.2"))]);
            assert_eq!(plan.hot, expected, "{path}");
        }
    }

    #[test]
    fn identity_and_files_require_restart() {
        for path in ["bridge.mac", "bifrost.state_file", "bifrost.cert_file"] {
            assert_restart_required(path);
        }
    }

    #[test]
    fn z2m_server_added_starts_backend() {
        let plan = plan_changes(vec![change(
            "z2m.new",
            Value::Null,
            json!({"url": "ws://x"}),
        )]);
        assert_eq!(plan.hot, [ApplyAction::StartBackend { name: "new".into() }]);
    }

    #[test]
    fn z2m_server_removed_stops_backend() {
        let plan = plan_changes(vec![change(
            "z2m.old",
            json!({"url": "ws://x"}),
            Value::Null,
        )]);
        assert_eq!(plan.hot, [ApplyAction::StopBackend { name: "old".into() }]);
    }

    #[test]
    fn z2m_server_settings_restart_backend() {
        for field in [
            "url",
            "token",
            "disable_tls_verify",
            "streaming_fps",
            "group_prefix",
        ] {
            let path = format!("z2m.server1.{field}");
            let plan = plan_changes(vec![change(&path, json!(1), json!(2))]);
            assert_eq!(
                plan.hot,
                [ApplyAction::RestartBackend {
                    name: "server1".into()
                }],
                "{path}"
            );
        }
    }

    #[test]
    fn repeated_actions_are_listed_once() {
        let plan = plan_changes(vec![
            change("bridge.ipaddress", json!("a"), json!("b")),
            change("bridge.netmask", json!("a"), json!("b")),
            change("z2m.server1.url", json!("a"), json!("b")),
            change("z2m.server1.group_prefix", json!("a"), json!("b")),
        ]);
        assert_eq!(plan.hot.len(), NETWORK_SERVICES.len() + 1);
    }

    #[test]
    fn diff_reports_typed_changes() {
        let old = minimal();
        let mut new = old.clone();
        new.bridge.name = "Renamed".into();
        new.bridge.http_port = 8080;
        new.z2m.servers.remove("server1");

        let paths: Vec<String> = diff(&old, &new).into_iter().map(|c| c.path).collect();
        assert_eq!(paths, ["bridge.http_port", "bridge.name", "z2m.server1"]);

        let plan = plan(&old, &new);
        assert_eq!(
            plan.hot,
            [
                restart_service(HTTP_SERVICE),
                update("bridge.name"),
                ApplyAction::StopBackend {
                    name: "server1".into()
                },
            ]
        );
    }

    #[test]
    fn diff_redacts_url_tokens() {
        let old = minimal();
        let mut new = old.clone();
        new.z2m.servers.get_mut("server1").unwrap().url =
            "ws://10.0.0.100:8080/api?token=s3cret".parse().unwrap();

        let changes = diff(&old, &new);

        assert_eq!(changes.len(), 1);
        assert!(changes[0].secret);
        assert_eq!(
            changes[0].after,
            json!(format!("ws://10.0.0.100:8080/api?token={REDACTED}"))
        );
        assert!(!serde_json::to_string(&changes).unwrap().contains("s3cret"));
    }

    #[test]
    fn diff_redacts_nested_secrets() {
        let old = minimal();
        let mut new = old.clone();
        let mut server = new.z2m.servers["server1"].clone();
        server.url = "wss://other/api?token=hunter2".parse().unwrap();
        new.z2m.servers.insert("other".into(), server);

        let changes = diff(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "z2m.other");
        assert!(changes[0].secret);
        assert!(!serde_json::to_string(&changes).unwrap().contains("hunter2"));
    }

    #[test]
    fn plain_changes_are_not_secret() {
        let old = minimal();
        let mut new = old.clone();
        new.bridge.name = "Renamed".into();

        let changes = diff(&old, &new);
        assert!(!changes[0].secret);
        assert_eq!(changes[0].before, json!("Bifrost"));
        assert_eq!(changes[0].after, json!("Renamed"));
    }
}
