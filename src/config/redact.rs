//! Redaction of secrets in configuration documents.
//!
//! Secrets are write-only: they are replaced with [`REDACTED`] wherever a
//! configuration leaves the process (API responses, diffs, logs).

use serde_json::Value;

use crate::config::{AppConfig, Z2mServer};

/// Keys whose values are secrets, wherever they appear in the document.
pub const SECRET_KEYS: &[&str] = &["token", "password", "password_hash"];

/// Keys holding a url that may carry a `token=` query parameter.
pub const URL_KEYS: &[&str] = &["url"];

/// Placeholder shown instead of a secret value.
pub const REDACTED: &str = "<<REDACTED>>";

#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    SECRET_KEYS.contains(&key)
}

/// Redact every secret in `value`, recursively. A secret that is set becomes
/// [`REDACTED`]; an unset (`null`) secret stays `null`, so a reader can tell
/// the two apart without learning the value.
pub fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if is_secret_key(key) {
                    if !child.is_null() {
                        *child = Value::String(REDACTED.to_string());
                    }
                } else if URL_KEYS.contains(&key.as_str()) {
                    if let Value::String(url) = child {
                        *url = Z2mServer::sanitize_url(url);
                    }
                } else {
                    redact(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

/// A redacted copy of `value`.
#[must_use]
pub fn redacted(value: &Value) -> Value {
    let mut copy = value.clone();
    redact(&mut copy);
    copy
}

/// `config` as a redacted document, safe to hand out.
pub fn redacted_config(config: &AppConfig) -> serde_json::Result<Value> {
    let mut value = serde_json::to_value(config)?;
    redact(&mut value);
    Ok(value)
}

/// A redacted copy of `config`, for interfaces typed as [`AppConfig`].
pub fn redacted_app_config(config: &AppConfig) -> serde_json::Result<AppConfig> {
    serde_json::from_value(redacted_config(config)?)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::config::redact::{REDACTED, redacted};

    #[test]
    fn secret_keys_are_redacted_at_any_depth() {
        let value = json!({
            "z2m": {"one": {"token": "abc"}},
            "auth": {"users": [{"name": "admin", "password_hash": "$argon2id$x"}]},
        });

        let expected = json!({
            "z2m": {"one": {"token": REDACTED}},
            "auth": {"users": [{"name": "admin", "password_hash": REDACTED}]},
        });
        assert_eq!(redacted(&value), expected);
    }

    #[test]
    fn unset_secret_stays_null() {
        let value = json!({"token": Value::Null});
        assert_eq!(redacted(&value), value);
    }

    #[test]
    fn url_token_is_redacted() {
        let value = json!({"url": "ws://host:8080/api?token=s3cret"});
        let expected = json!({"url": format!("ws://host:8080/api?token={REDACTED}")});
        assert_eq!(redacted(&value), expected);
    }

    #[test]
    fn url_default_token_is_kept() {
        let value = json!({"url": "ws://host:8080/api?token=your-secret-token"});
        assert_eq!(redacted(&value), value);
    }

    #[test]
    fn other_values_are_untouched() {
        let value = json!({"bridge": {"name": "Bifrost", "http_port": 80}});
        assert_eq!(redacted(&value), value);
    }
}
