//! What a sensor reports, and how a z2m reading becomes a Hue value.
//!
//! A z2m binary expose says which JSON value means "active": `occupancy` is
//! `true` on most firmware and `false` on some, and a contact sensor reports
//! `false` for an open door. Everything downstream — the inventory reading,
//! the Hue resource, the v1 row — wants one meaning, so the raw value is
//! turned into "is the thing happening" here, once, using the expose the
//! device declares.

use serde_json::{Value, json};

use bifrost_api::inventory::DeviceType;
use z2m::api::{Device, Expose};

/// The classes this wave serves, and the property each one reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorKind {
    Motion,
    Temperature,
    Illuminance,
    Contact,
    Remote,
}

impl SensorKind {
    /// The class a z2m device belongs to, if this wave serves it.
    ///
    /// The order matches [`crate::inventory::classify`]: a device that
    /// reports motion is a motion sensor whatever else it also reports.
    #[must_use]
    pub fn of(device: &Device) -> Option<Self> {
        let kind = match crate::inventory::classify(device).0 {
            DeviceType::Motion => Self::Motion,
            DeviceType::Temperature => Self::Temperature,
            DeviceType::Illuminance => Self::Illuminance,
            DeviceType::Contact => Self::Contact,
            DeviceType::Remote => Self::Remote,
            _ => return None,
        };
        Some(kind)
    }

    /// The z2m property this class reads.
    #[must_use]
    pub const fn property(self) -> &'static str {
        match self {
            Self::Motion => "occupancy",
            Self::Temperature => "temperature",
            Self::Illuminance => "illuminance",
            Self::Contact => "contact",
            Self::Remote => "action",
        }
    }

    /// Whether the property is a binary the expose decides the meaning of.
    #[must_use]
    pub const fn is_binary(self) -> bool {
        matches!(self, Self::Motion | Self::Contact)
    }

    /// The classes that own a Hue sensor resource, which is every one but
    /// the remote: a remote is a `Button`.
    #[must_use]
    pub const fn is_sensor(self) -> bool {
        !matches!(self, Self::Remote)
    }
}

/// Whether the raw z2m value means "active" for `property`.
///
/// The expose's `value_on` decides; a device that does not declare one keeps
/// the raw value, which is what z2m itself would show.
#[must_use]
pub fn is_active(device: &Device, property: &str, raw: bool) -> bool {
    value_on(device, property).map_or(raw, |on| raw == on)
}

/// The JSON value the device declares as "on" for `property`.
fn value_on(device: &Device, property: &str) -> Option<bool> {
    for expose in flatten(device.exposes()) {
        // a nested `if` rather than a let-chain: the image builds on the
        // workspace's minimum Rust, where `if let … && …` is not stable yet
        if let Expose::Binary(binary) = expose {
            if binary.base.property.as_deref() == Some(property) {
                return binary.value_on.as_bool();
            }
        }
    }
    None
}

/// The unit a numeric property is reported in, where the expose names one.
#[must_use]
pub fn unit_of(device: &Device, property: &str) -> Option<String> {
    for expose in flatten(device.exposes()) {
        if let Expose::Numeric(numeric) = expose {
            if numeric.base.property.as_deref() == Some(property) {
                return numeric.unit.clone();
            }
        }
    }
    None
}

/// Every expose of a device, features included.
fn flatten(exposes: &[Expose]) -> Vec<&Expose> {
    let mut out = Vec::new();
    let mut pending: Vec<&Expose> = exposes.iter().rev().collect();
    while let Some(expose) = pending.pop() {
        out.push(expose);
        pending.extend(expose.base().features.iter().rev());
    }
    out
}

/// A lux reading as the `light_level` a Hue bridge reports.
///
/// The cast is exact: the value is clamped into `u32`'s range and rounded
/// first, so neither truncation nor a sign change can occur.
///
/// The bridge's own scale is `lux = 10^((light_level - 1) / 10000)`, so the
/// inverse is exact rather than an approximation. Below 1 lx the scale runs
/// negative; the resource clamps at zero, which is "darker than it can say".
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn light_level(lux: f64) -> u32 {
    if lux <= 1.0 {
        return 0;
    }
    let level = 10_000.0_f64.mul_add(lux.log10(), 1.0);
    // the wire field is a u32; anything above is brighter than the scale's
    // top and clamps there
    level.clamp(0.0, f64::from(u32::MAX)).round() as u32
}

/// A lux reading back out of a Hue `light_level`, for the round-trip test.
#[must_use]
pub fn lux_from_light_level(level: u32) -> f64 {
    10.0_f64.powf((f64::from(level) - 1.0) / 10_000.0)
}

/// A battery percentage as the wire carries it.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn battery_percent(level: f64) -> u8 {
    level.clamp(0.0, 100.0).round() as u8
}

/// The v1 `buttonevent` code for a z2m action.
///
/// v1 encodes a press as `<button><event>`: 1000 initial press, 1001 hold,
/// 1002 short release, 1003 long release. z2m names the gesture instead of
/// numbering it, so the name is what maps; a name with no Hue equivalent
/// becomes a short release, which is what most remotes send.
#[must_use]
pub fn buttonevent(action: &str) -> u32 {
    let action = action.to_ascii_lowercase();
    if action.contains("release") {
        1003
    } else if action.contains("hold") || action.contains("long") {
        1001
    } else if action.contains("press") {
        1000
    } else {
        1002
    }
}

/// The `power_state` a `DevicePower` resource carries.
#[must_use]
pub fn power_state(battery: Option<u8>) -> Value {
    battery.map_or_else(
        || {
            json!({
                "battery_level": Value::Null,
                "battery_state": "normal",
            })
        },
        |level| {
            json!({
                "battery_level": level,
                "battery_state": battery_state(level),
            })
        },
    )
}

/// The word the bridge uses for a battery level.
#[must_use]
pub const fn battery_state(level: u8) -> &'static str {
    if level <= 10 {
        "critical"
    } else if level <= 20 {
        "low"
    } else {
        "normal"
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn device(exposes: &Value) -> Device {
        let device = json!({
            "description": null,
            "disabled": false,
            "endpoints": {},
            "friendly_name": "Hall",
            "ieee_address": "0x00158d0000000020",
            "interview_completed": true,
            "interviewing": false,
            "manufacturer": "LUMI",
            "model_id": "RTCGQ11LM",
            "network_address": 1234,
            "type": "EndDevice",
            "definition": {
                "model": "RTCGQ11LM",
                "vendor": "LUMI",
                "description": "Aqara motion sensor",
                "exposes": exposes,
                "options": [],
                "supports_ota": false,
            },
        });
        serde_json::from_str(&device.to_string()).unwrap()
    }

    #[test]
    fn a_contact_sensor_that_reports_false_for_open_is_read_as_open() {
        // MCCGQ11LM declares value_on: false
        let device = device(&json!([{
            "type": "binary",
            "property": "contact",
            "access": 1,
            "value_on": false,
            "value_off": true,
        }]));

        assert!(is_active(&device, "contact", false));
        assert!(!is_active(&device, "contact", true));
    }

    #[test]
    fn a_motion_sensor_that_reports_true_for_motion_keeps_its_value() {
        let device = device(&json!([{
            "type": "binary",
            "property": "occupancy",
            "access": 1,
            "value_on": true,
            "value_off": false,
        }]));

        assert!(is_active(&device, "occupancy", true));
        assert!(!is_active(&device, "occupancy", false));
    }

    #[test]
    fn a_device_without_a_declared_value_on_keeps_the_raw_value() {
        let device = device(&json!([{"type": "numeric", "property": "temperature"}]));
        assert!(is_active(&device, "occupancy", true));
        assert!(!is_active(&device, "occupancy", false));
    }

    #[test]
    fn the_light_level_scale_round_trips_through_lux() {
        for lux in [1.0, 2.0, 10.0, 100.0, 1000.0, 10_000.0] {
            let level = light_level(lux);
            let back = lux_from_light_level(level);
            // rounding to an integer level costs at most half a step, which
            // is under 0.02 % of the reading at any real brightness
            assert!(
                (back - lux).abs() / lux < 0.001,
                "{lux} lx came back as {back} lx from {level}"
            );
        }
        assert_eq!(light_level(0.0), 0);
        assert_eq!(light_level(0.5), 0);
    }

    #[test]
    fn a_gesture_maps_to_the_v1_buttonevent_code() {
        assert_eq!(buttonevent("single"), 1002);
        assert_eq!(buttonevent("initial_press"), 1000);
        assert_eq!(buttonevent("hold"), 1001);
        assert_eq!(buttonevent("long_release"), 1003);
        assert_eq!(buttonevent("something_new"), 1002);
    }

    #[test]
    fn a_battery_level_names_its_state() {
        assert_eq!(battery_percent(87.4), 87);
        assert_eq!(battery_percent(-3.0), 0);
        assert_eq!(battery_percent(300.0), 100);
        assert_eq!(battery_state(8), "critical");
        assert_eq!(battery_state(12), "low");
        assert_eq!(battery_state(87), "normal");
        assert_eq!(power_state(Some(12))["battery_state"], "low");
    }

    #[test]
    fn a_class_is_recognised_from_the_exposes_alone() {
        assert_eq!(
            SensorKind::of(&device(&json!([{
                "type": "binary", "property": "occupancy", "access": 1,
                "value_on": true, "value_off": false,
            }]))),
            Some(SensorKind::Motion)
        );
        assert_eq!(
            SensorKind::of(&device(&json!([{"type": "light", "features": []}]))),
            None
        );
    }
}
