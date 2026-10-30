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

/// Resolve write-only secrets in an incoming document against `existing`.
///
/// An absent, `null` or [`REDACTED`] secret keeps the existing value; `""`
/// clears it. A url that comes back exactly as it was handed out (with its
/// token redacted) keeps the existing url.
pub fn merge_secrets(incoming: &mut Value, existing: &Value) {
    keep_existing(incoming, existing);
    clear_secrets(incoming);
}

fn keep_existing(incoming: &mut Value, existing: &Value) {
    let (Value::Object(incoming), Value::Object(existing)) = (incoming, existing) else {
        return;
    };

    for (key, old) in existing {
        if is_secret_key(key) {
            let keep = match incoming.get(key) {
                None | Some(Value::Null) => true,
                Some(Value::String(text)) => text == REDACTED,
                Some(_) => false,
            };
            if keep && !old.is_null() {
                incoming.insert(key.clone(), old.clone());
            }
        } else if URL_KEYS.contains(&key.as_str()) {
            if let (Some(Value::String(new)), Value::String(old)) = (incoming.get_mut(key), old) {
                if *new != *old && *new == Z2mServer::sanitize_url(old) {
                    new.clone_from(old);
                }
            }
        } else if let Some(child) = incoming.get_mut(key) {
            keep_existing(child, old);
        }
    }
}

/// Drop secrets set to `""` (cleared), and placeholders with nothing to keep.
fn clear_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, child| {
                !(is_secret_key(key) && matches!(child.as_str(), Some("" | REDACTED)))
            });
            map.values_mut().for_each(clear_secrets);
        }
        Value::Array(items) => items.iter_mut().for_each(clear_secrets),
        _ => {}
    }
}

/// Apply a JSON merge patch (RFC 7386) to `target`.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };

    if !target.is_object() {
        *target = Value::Object(serde_json::Map::new());
    }
    let Value::Object(map) = target else {
        return;
    };

    for (key, value) in patch {
        if value.is_null() {
            map.remove(key);
        } else {
            merge_patch(map.entry(key.clone()).or_insert(Value::Null), value);
        }
    }
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

    use crate::config::redact::{REDACTED, merge_patch, merge_secrets, redacted};

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

    fn existing() -> Value {
        json!({
            "z2m": {
                "one": {"url": "ws://host/api?token=old-url-token", "token": "old"},
            },
            "auth": {"password_hash": "$argon2id$old"},
        })
    }

    #[test]
    fn absent_or_null_secret_keeps_existing() {
        let mut incoming =
            json!({"z2m": {"one": {"url": "ws://host/api"}}, "auth": {"password_hash": null}});
        merge_secrets(&mut incoming, &existing());

        assert_eq!(incoming["z2m"]["one"]["token"], "old");
        assert_eq!(incoming["auth"]["password_hash"], "$argon2id$old");
    }

    #[test]
    fn redacted_document_round_trips_to_existing() {
        let mut incoming = redacted(&existing());
        merge_secrets(&mut incoming, &existing());

        assert_eq!(incoming, existing());
    }

    #[test]
    fn empty_secret_clears() {
        let mut incoming = json!({"z2m": {"one": {"url": "ws://host/api", "token": ""}}});
        merge_secrets(&mut incoming, &existing());

        assert_eq!(incoming, json!({"z2m": {"one": {"url": "ws://host/api"}}}));
    }

    #[test]
    fn new_secret_replaces_existing() {
        let mut incoming = json!({"z2m": {"one": {"url": "ws://host/api", "token": "new"}}});
        merge_secrets(&mut incoming, &existing());

        assert_eq!(incoming["z2m"]["one"]["token"], "new");
    }

    #[test]
    fn placeholder_without_existing_value_is_dropped() {
        let mut incoming = json!({"z2m": {"two": {"url": "ws://x", "token": REDACTED}}});
        merge_secrets(&mut incoming, &existing());

        assert_eq!(incoming, json!({"z2m": {"two": {"url": "ws://x"}}}));
    }

    #[test]
    fn changed_url_is_kept() {
        let mut incoming = json!({"z2m": {"one": {"url": "ws://other/api?token=new"}}});
        merge_secrets(&mut incoming, &existing());

        assert_eq!(incoming["z2m"]["one"]["url"], "ws://other/api?token=new");
    }

    #[test]
    fn merge_patch_follows_rfc_7386() {
        let mut target = json!({"a": "b", "c": {"d": "e", "f": "g"}});
        merge_patch(&mut target, &json!({"a": "z", "c": {"f": null}, "h": [1]}));

        assert_eq!(target, json!({"a": "z", "c": {"d": "e"}, "h": [1]}));
    }

    #[test]
    fn merge_patch_null_secret_keeps_existing() {
        let mut target = existing();
        merge_patch(&mut target, &json!({"z2m": {"one": {"token": null}}}));
        merge_secrets(&mut target, &existing());

        assert_eq!(target, existing());
    }
}
