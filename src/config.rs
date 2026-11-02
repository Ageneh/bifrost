pub mod diff;
pub mod redact;
pub mod validate;

use camino::Utf8Path;
use config::{Config, ConfigError};

pub use bifrost_api::config::*;

pub fn parse(filename: &Utf8Path) -> Result<AppConfig, ConfigError> {
    let settings = Config::builder()
        .set_default("bifrost.state_file", "state.yaml")?
        .set_default("bifrost.cert_file", "cert.pem")?
        .set_default("bridge.http_port", 80)?
        .set_default("bridge.https_port", 443)?
        .set_default("bridge.entm_port", 2100)?
        .add_source(config::File::with_name(filename.as_str()))
        .build()?;

    settings.try_deserialize()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::config::parse;
    use crate::config_store::ConfigStore;
    use crate::testutil::TempDir;

    /// A config.yaml as written for earlier versions: comments, no ports, no
    /// `bifrost` section, a token in the url query, and settings this
    /// version does not know at every level.
    const LEGACY_CONFIG: &str = "\
# my bridge
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 10.12.0.20
  netmask: 255.255.255.0
  gateway: 10.12.0.1
  timezone: Europe/Copenhagen
  futureBridgeSetting: true

z2m:
  server1:
    url: ws://10.0.0.100:8080/api?token=legacy-secret
    group_prefix: bifrost_
    future_server_setting: [1, 2]

rooms:
  office_group:
    name: Office
    icon: office
    future_room_setting: x

future_section:
  nested:
    value: 42
";

    #[test]
    fn legacy_config_loads_with_defaults() {
        let dir = TempDir::new();
        let config = parse(&dir.write("config.yaml", LEGACY_CONFIG)).unwrap();

        assert_eq!(config.bridge.http_port, 80);
        assert_eq!(config.bridge.https_port, 443);
        assert_eq!(config.bridge.entm_port, 2100);
        assert_eq!(config.bifrost.state_file, "state.yaml");
        assert_eq!(config.bifrost.cert_file, "cert.pem");

        let server = &config.z2m.servers["server1"];
        assert_eq!(server.token, None);
        assert_eq!(
            server.get_url().as_str(),
            "ws://10.0.0.100:8080/api?token=legacy-secret"
        );
    }

    #[test]
    fn legacy_config_upgrade_keeps_unknown_settings_and_migrates_token() {
        let dir = TempDir::new();
        let path = dir.write("config.yaml", LEGACY_CONFIG);
        let store = ConfigStore::load(&path).unwrap();
        let url_before = store.active().z2m.servers["server1"].get_url();

        let mut draft = store.active();
        draft.bridge.name = "Upgraded".into();
        store.set_draft(0, draft).unwrap();
        let outcome = store.apply(1).unwrap();

        // the migration alone is not reported as a change
        let paths: Vec<&str> = outcome.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["bridge.name"]);

        assert_eq!(
            fs::read_to_string(&store.paths().backup).unwrap(),
            LEGACY_CONFIG
        );

        let written = fs::read_to_string(&path).unwrap();
        for key in [
            "futureBridgeSetting",
            "future_server_setting",
            "future_room_setting",
            "future_section",
        ] {
            assert!(written.contains(key), "{key} lost:\n{written}");
        }

        let reloaded = parse(&path).unwrap();
        let server = &reloaded.z2m.servers["server1"];
        assert_eq!(server.token.as_deref(), Some("legacy-secret"));
        assert_eq!(server.url.as_str(), "ws://10.0.0.100:8080/api");
        assert_eq!(server.get_url(), url_before);
        assert_eq!(reloaded.bridge.name, "Upgraded");
        assert_eq!(
            reloaded.unknown["future_section"]["nested"]["value"],
            serde_json::json!(42)
        );
    }

    #[test]
    fn saved_config_round_trips_unchanged() {
        let dir = TempDir::new();
        let path = dir.write("config.yaml", LEGACY_CONFIG);
        let store = ConfigStore::load(&path).unwrap();
        store.set_draft(0, store.active()).unwrap();
        store.apply(1).unwrap();
        let first = fs::read_to_string(&path).unwrap();

        let store = ConfigStore::load(&path).unwrap();
        store.set_draft(store.revision(), store.active()).unwrap();
        store.apply(store.revision()).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), first);
    }

    #[test]
    fn debug_output_hides_secrets() {
        let dir = TempDir::new();
        let mut config = parse(&dir.write("config.yaml", LEGACY_CONFIG)).unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("legacy-secret"), "{debug}");

        config.migrate_url_tokens();
        let debug = format!("{config:?}");
        assert!(!debug.contains("legacy-secret"), "{debug}");
    }
}
