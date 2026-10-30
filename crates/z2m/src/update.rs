use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use hue::api::{LightGradientUpdate, On};
use hue::xy::XY;

use crate::hexcolor::HexColor;

#[allow(clippy::pub_underscore_fields)]
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DeviceUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<DeviceState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_temp: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_mode: Option<DeviceColorMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<DeviceColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gradient: Option<Vec<HexColor>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkquality: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_options: Option<ColorOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_temp_startup: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level_config: Option<LevelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub elapsed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub power_on_behavior: Option<PowerOnBehavior>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    #[serde(default)]
    pub update: HashMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<DeviceEffect>,

    /* sensor readings, typed so a caller never reads the flatten map */
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "de::opt_bool_like")]
    pub occupancy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub illuminance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "de::opt_bool_like")]
    pub contact: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "de::opt_text_like")]
    pub action: Option<String>,

    /* all other fields */
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    #[serde(default, flatten)]
    pub __: HashMap<String, Value>,
}

impl DeviceUpdate {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_state(self, state: Option<bool>) -> Self {
        Self {
            state: state.map(|on| {
                if on {
                    DeviceState::On
                } else {
                    DeviceState::Off
                }
            }),
            ..self
        }
    }

    #[must_use]
    pub fn with_brightness(self, brightness: Option<f64>) -> Self {
        Self {
            brightness: brightness.map(|b| b.clamp(1.0, 254.0)),
            ..self
        }
    }

    #[must_use]
    pub fn with_color_temp(self, mirek: Option<u16>) -> Self {
        Self {
            color_temp: mirek,
            ..self
        }
    }

    #[must_use]
    pub fn with_color_xy(self, xy: Option<XY>) -> Self {
        Self {
            color: xy.map(DeviceColor::xy),
            ..self
        }
    }

    #[must_use]
    pub fn with_gradient(self, grad: Option<LightGradientUpdate>) -> Self {
        Self {
            gradient: grad.map(|g| {
                g.points
                    .iter()
                    .map(|p| {
                        let [r, g, b] = p.color.xy.to_rgb(255.0);
                        HexColor::new(r, g, b)
                    })
                    .collect()
            }),
            ..self
        }
    }

    #[must_use]
    pub fn with_effect(self, effect: DeviceEffect) -> Self {
        Self {
            effect: Some(effect),
            ..self
        }
    }

    #[must_use]
    pub fn with_transition(self, transition: Option<f64>) -> Self {
        Self { transition, ..self }
    }
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DeviceColor {
    #[allow(dead_code)]
    #[serde(skip_serializing)]
    h: Option<f64>,
    #[allow(dead_code)]
    #[serde(skip_serializing)]
    s: Option<f64>,

    pub hue: Option<f64>,
    pub saturation: Option<f64>,

    #[serde(flatten)]
    pub xy: Option<XY>,
}

impl DeviceColor {
    #[must_use]
    pub const fn xy(xy: XY) -> Self {
        Self {
            h: None,
            s: None,
            hue: None,
            saturation: None,
            xy: Some(xy),
        }
    }

    #[must_use]
    pub const fn hs(h: f64, s: f64) -> Self {
        Self {
            h: None,
            s: None,
            hue: Some(h),
            saturation: Some(s),
            xy: None,
        }
    }
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub enum PowerOnBehavior {
    #[default]
    Unknown,

    #[serde(rename = "on")]
    On,

    #[serde(rename = "off")]
    Off,

    #[serde(rename = "previous")]
    Previous,
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ColorOptions {
    pub execute_if_off: bool,
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct LevelConfig {
    pub execute_if_off: Option<bool>,
    pub on_off_transition_time: Option<u16>,
    pub on_transition_time: Option<u16>,
    pub off_transition_time: Option<u16>,
    pub current_level_startup: Option<CurrentLevelStartup>,
    pub on_level: Option<OnLevel>,
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum CurrentLevelStartup {
    Previous,
    Minimum,
    #[serde(untagged)]
    Value(u8),
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum OnLevel {
    Previous,
    #[serde(untagged)]
    Value(u8),
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceColorMode {
    ColorTemp,
    Hs,
    Xy,
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum DeviceState {
    On,
    Off,
    Lock,
    Unlock,
}

impl From<DeviceState> for On {
    fn from(value: DeviceState) -> Self {
        Self {
            on: value == DeviceState::On,
        }
    }
}

#[derive(Copy, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceEffect {
    Blink,
    Breathe,
    Okay,
    ChannelChange,
    FinishEffect,
    StopEffect,
}

/// Deserializers for the values zigbee2mqtt does not keep to one JSON type.
///
/// A sensor reports `occupancy` as `true` on most firmware and as `"ON"` on
/// some, and `action` is a string on every one; reading them through a
/// `serde_json::Value` first costs one allocation per message and buys a
/// parser that does not drop a reading over a spelling.
mod de {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    /// A boolean as z2m spells it: `true`/`false`, `1`/`0`, `"ON"`/`"OFF"`.
    pub fn opt_bool_like<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<Value>::deserialize(deserializer)?;
        Ok(value.and_then(|value| match value {
            Value::Bool(on) => Some(on),
            Value::Number(number) => number.as_f64().map(|number| number != 0.0),
            Value::String(text) => match text.to_ascii_lowercase().as_str() {
                "true" | "on" | "1" => Some(true),
                "false" | "off" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        }))
    }

    /// A string that a device may report as a number instead.
    pub fn opt_text_like<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<Value>::deserialize(deserializer)?;
        Ok(value.and_then(|value| match value {
            Value::String(text) => Some(text),
            Value::Null => None,
            other => Some(other.to_string()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Parse one payload the way the backend does.
    fn update(payload: serde_json::Value) -> DeviceUpdate {
        serde_json::from_value(payload).unwrap()
    }

    #[test]
    fn a_sensor_reading_lands_in_its_own_field() {
        let update = update(json!({
            "occupancy": true,
            "illuminance": 312.0,
            "temperature": 21.4,
            "contact": false,
            "battery": 87,
            "voltage": 2900,
        }));

        assert_eq!(update.occupancy, Some(true));
        assert_eq!(update.illuminance, Some(312.0));
        assert_eq!(update.temperature, Some(21.4));
        assert_eq!(update.contact, Some(false));
        assert_eq!(update.battery, Some(87.0));
        assert_eq!(update.voltage, Some(2900.0));
        // nothing spills into the flatten map any more
        assert!(update.__.is_empty());
    }

    #[test]
    fn a_binary_reported_as_a_string_or_a_number_is_still_read() {
        assert_eq!(update(json!({"occupancy": "ON"})).occupancy, Some(true));
        assert_eq!(update(json!({"occupancy": "off"})).occupancy, Some(false));
        assert_eq!(update(json!({"occupancy": 1})).occupancy, Some(true));
        assert_eq!(update(json!({"occupancy": 0})).occupancy, Some(false));
        // an unknown spelling is a missing reading, not an error
        assert_eq!(update(json!({"occupancy": "maybe"})).occupancy, None);
    }

    #[test]
    fn an_action_is_a_string_whatever_the_device_sends() {
        assert_eq!(
            update(json!({"action": "single"})).action.as_deref(),
            Some("single")
        );
        assert_eq!(update(json!({"action": 2})).action.as_deref(), Some("2"));
    }

    #[test]
    fn a_light_payload_carries_no_sensor_reading() {
        let update = update(json!({"state": "ON", "brightness": 128}));
        assert_eq!(update.occupancy, None);
        assert_eq!(update.temperature, None);
        assert_eq!(update.action, None);
    }
}
