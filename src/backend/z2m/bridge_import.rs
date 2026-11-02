use std::collections::HashSet;

use chrono::Utc;
use maplit::btreeset;
use serde_json::json;
use uuid::Uuid;

use hue::api::{
    BridgeHome, Button, ButtonData, ButtonMetadata, ButtonReport, Contact, DeviceArchetype,
    DevicePower, DeviceProductData, Entertainment, EntertainmentSegment, EntertainmentSegments,
    GroupedLight, Light, LightEffects, LightEffectsV2, LightLevel, LightMetadata, Metadata, Motion,
    RType, Resource, ResourceLink, Room, RoomArchetype, RoomMetadata, Scene, SceneActive,
    SceneMetadata, SceneRecall, SceneStatus, Stub, Taurus, Temperature, ZigbeeConnectivity,
    ZigbeeConnectivityStatus,
};
use hue::scene_icons;
use z2m::api::ExposeLight;
use z2m::convert::{
    ExtractColorTemperature, ExtractDeviceProductData, ExtractDimming, ExtractLightColor,
    ExtractLightGradient,
};

use crate::backend::z2m::Z2mBackend;
use crate::backend::z2m::sensor::{self, SensorKind};
use crate::error::ApiResult;
use crate::model::state::AuxData;

impl Z2mBackend {
    #[allow(clippy::too_many_lines)]
    pub async fn add_light(
        &mut self,
        apidev: &z2m::api::Device,
        expose: &ExposeLight,
    ) -> ApiResult<()> {
        let name = &apidev.friendly_name;

        // a name override replaces what the Hue API shows, never the topic
        // z2m is addressed by
        let ieee = apidev.ieee_address.to_string();
        let override_name = self
            .store
            .active()
            .device(&ieee)
            .and_then(|device| device.name.clone());
        let shown_name = override_name.unwrap_or_else(|| name.clone());

        let link_device = RType::Device.deterministic(&apidev.ieee_address);
        let link_light = RType::Light.deterministic(&apidev.ieee_address);
        let link_enttm = RType::Entertainment.deterministic(&apidev.ieee_address);
        let link_taurus = RType::Taurus.deterministic(&apidev.ieee_address);
        let link_zigcon = RType::ZigbeeConnectivity.deterministic(&apidev.ieee_address);

        let product_data = DeviceProductData::guess_from_device(apidev);
        let metadata = LightMetadata::new(product_data.product_archetype.clone(), &shown_name);

        let effects =
            apidev.manufacturer.as_deref() == Some(DeviceProductData::SIGNIFY_MANUFACTURER_NAME);
        let gradient = apidev.expose_gradient();

        let dev = hue::api::Device {
            product_data,
            metadata: metadata.clone().into(),
            services: btreeset![link_zigcon, link_light, link_enttm, link_taurus],
            identify: Some(Stub),
            usertest: None,
        };

        self.map.insert(name.clone(), link_light);
        self.rmap.insert(link_device, name.clone());
        self.rmap.insert(link_light, name.clone());

        let mut light = Light::new(link_device, metadata.clone());

        light.dimming = expose
            .feature("brightness")
            .and_then(ExtractDimming::extract_from_expose);
        log::trace!("Detected dimming: {:?}", &light.dimming);

        light.color_temperature = expose
            .feature("color_temp")
            .and_then(ExtractColorTemperature::extract_from_expose);
        log::trace!("Detected color temperature: {:?}", &light.color_temperature);

        light.color = expose
            .feature("color_xy")
            .and_then(ExtractLightColor::extract_from_expose);
        log::trace!("Detected color: {:?}", &light.color);

        light.gradient = gradient.and_then(ExtractLightGradient::extract_from_expose);
        log::trace!("Detected gradient support: {:?}", &light.gradient);

        if effects {
            log::trace!("Detected Hue light: enabling effects");
            light.effects = Some(LightEffects::all());
            light.effects_v2 = Some(LightEffectsV2::all());
        }

        let segments = if gradient.is_some() {
            EntertainmentSegments {
                configurable: false,
                max_segments: 10,
                segments: (0..7)
                    .map(|x| EntertainmentSegment {
                        start: x,
                        length: 1,
                    })
                    .collect(),
            }
        } else {
            EntertainmentSegments {
                configurable: false,
                max_segments: 1,
                segments: vec![EntertainmentSegment {
                    start: 0,
                    length: 1,
                }],
            }
        };

        // FIXME: This should be feature-detected, not always enabled
        let enttm = Entertainment {
            equalizer: true,
            owner: link_device,
            proxy: true,
            renderer: true,
            max_streams: None,
            renderer_reference: Some(link_light),
            segments: Some(segments),
        };

        // FIXME: The Taurus objects are seen on Hue Entertainment devices on a
        // real hue bridge, but nobody knows what it does. Some clients seem to
        // want them present, though.
        let taurus = Taurus {
            capabilities: vec![
                "sensor".to_string(),
                "collector".to_string(),
                "sync".to_string(),
            ],
            owner: link_device,
        };

        let zigcon = ZigbeeConnectivity {
            channel: None,
            extended_pan_id: None,
            mac_address: apidev.ieee_address.to_string(),
            owner: link_device,
            status: ZigbeeConnectivityStatus::Connected,
        };

        let mut res = self.state.lock().await;
        res.aux_set(&link_light, AuxData::new().with_topic(name));
        res.add(&link_device, Resource::Device(dev))?;
        res.add(&link_light, Resource::Light(light))?;

        // a light that is already known keeps its id, so a name override
        // arrives as an update; adding it again would leave the old name
        res.update(&link_light.rid, |known: &mut Light| {
            known.metadata = metadata.clone();
        })?;
        res.update(&link_device.rid, |known: &mut hue::api::Device| {
            known.metadata = metadata.clone().into();
        })?;
        res.add(&link_enttm, Resource::Entertainment(enttm))?;
        res.add(&link_taurus, Resource::Taurus(taurus))?;
        res.add(&link_zigcon, Resource::ZigbeeConnectivity(zigcon))?;
        drop(res);

        Ok(())
    }

    /// Import one sensor or remote: the `Device`, the resource its class
    /// owns, and the two companions every battery-powered device has.
    ///
    /// The companions are not optional. A Hue client that finds a `motion`
    /// resource with no `device_power` beside it either hides the sensor or
    /// shows it without a battery, and that is the failure mode this import
    /// exists to prevent.
    #[allow(clippy::too_many_lines)]
    pub async fn add_sensor(
        &mut self,
        apidev: &z2m::api::Device,
        kind: SensorKind,
    ) -> ApiResult<()> {
        let name = &apidev.friendly_name;
        let ieee = apidev.ieee_address.to_string();

        let override_name = self
            .store
            .active()
            .device(&ieee)
            .and_then(|device| device.name.clone());
        let shown_name = override_name.unwrap_or_else(|| name.clone());

        let link_device = RType::Device.deterministic(&apidev.ieee_address);
        let link_zigcon = RType::ZigbeeConnectivity.deterministic(&apidev.ieee_address);
        let link_power = RType::DevicePower.deterministic(&apidev.ieee_address);
        let link_service = match kind {
            SensorKind::Motion => RType::Motion.deterministic(&apidev.ieee_address),
            SensorKind::Temperature => RType::Temperature.deterministic(&apidev.ieee_address),
            SensorKind::Illuminance => RType::LightLevel.deterministic(&apidev.ieee_address),
            SensorKind::Contact => RType::Contact.deterministic(&apidev.ieee_address),
            SensorKind::Remote => RType::Button.deterministic(&apidev.ieee_address),
        };

        let metadata = Metadata::new(DeviceArchetype::UnknownArchetype, &shown_name);
        let dev = hue::api::Device {
            product_data: DeviceProductData::guess_from_device(apidev),
            metadata: metadata.clone(),
            services: btreeset![link_zigcon, link_power, link_service],
            identify: None,
            usertest: None,
        };

        // the topic is the key every update arrives on; the resource is what
        // it becomes
        self.map.insert(name.clone(), link_service);
        self.rmap.insert(link_device, name.clone());
        self.rmap.insert(link_service, name.clone());

        let power = DevicePower {
            owner: link_device,
            power_state: sensor::power_state(None),
        };
        let zigcon = ZigbeeConnectivity {
            channel: None,
            extended_pan_id: None,
            mac_address: apidev.ieee_address.to_string(),
            owner: link_device,
            status: ZigbeeConnectivityStatus::Connected,
        };

        let mut res = self.state.lock().await;
        res.aux_set(&link_service, AuxData::new().with_topic(name));
        res.add(&link_device, Resource::Device(dev))?;
        res.add(&link_zigcon, Resource::ZigbeeConnectivity(zigcon))?;
        res.add(&link_power, Resource::DevicePower(power))?;

        let initial = initial_resource(kind, link_device);
        res.add(&link_service, initial)?;

        // a device that is already known keeps its id and its readings, so a
        // re-import is an update of the name rather than a new sensor
        res.update(&link_device.rid, |known: &mut hue::api::Device| {
            known.metadata = metadata;
        })?;
        drop(res);

        Ok(())
    }

    pub async fn add_switch(&mut self, dev: &z2m::api::Device) -> ApiResult<()> {
        let name = &dev.friendly_name;

        let link_device = RType::Device.deterministic(&dev.ieee_address);
        let link_button = RType::Button.deterministic(&dev.ieee_address);
        let link_zbc = RType::ZigbeeConnectivity.deterministic(&dev.ieee_address);

        let dev = hue::api::Device {
            product_data: DeviceProductData::guess_from_device(dev),
            metadata: Metadata::new(DeviceArchetype::UnknownArchetype, "foo"),
            services: btreeset![link_button, link_zbc],
            identify: None,
            usertest: None,
        };

        self.map.insert(name.clone(), link_button);
        self.rmap.insert(link_button, name.clone());

        let mut res = self.state.lock().await;
        let button = Button {
            owner: link_device,
            metadata: ButtonMetadata { control_id: 0 },
            button: ButtonData {
                last_event: None,
                button_report: Some(ButtonReport {
                    updated: Utc::now(),
                    event: String::from("initial_press"),
                }),
                repeat_interval: Some(100),
                event_values: Some(json!(["initial_press", "repeat"])),
            },
        };

        let zbc = ZigbeeConnectivity {
            owner: link_device,
            mac_address: String::from("11:22:33:44:55:66:77:89"),
            status: ZigbeeConnectivityStatus::ConnectivityIssue,
            channel: Some(json!({
                "status": "set",
                "value": "channel_25",
            })),
            extended_pan_id: None,
        };

        res.add(&link_device, Resource::Device(dev))?;
        res.add(&link_button, Resource::Button(button))?;
        res.add(&link_zbc, Resource::ZigbeeConnectivity(zbc))?;
        drop(res);

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub async fn add_group(&mut self, grp: &z2m::api::Group) -> ApiResult<()> {
        let room_name;

        if let Some(ref prefix) = self.server.group_prefix {
            if let Some(name) = grp.friendly_name.strip_prefix(prefix) {
                room_name = name;
            } else {
                log::debug!(
                    "[{}] Ignoring room outside our prefix: {}",
                    self.name,
                    grp.friendly_name
                );
                return Ok(());
            }
        } else {
            room_name = &grp.friendly_name;
        }

        let link_room = RType::Room.deterministic(&grp.friendly_name);
        let link_glight = RType::GroupedLight.deterministic((link_room.rid, grp.id));

        let topic = grp.friendly_name.to_string();

        // the live configuration, so room changes apply without a restart
        let room_conf = self.store.active().rooms.get(&topic).cloned();

        // a drafted membership replaces what the group holds; absent, the
        // group is the membership
        let drafted = room_conf.as_ref().and_then(|room| room.devices.as_ref());
        let children = drafted.map_or_else(
            || {
                grp.members
                    .iter()
                    .map(|f| RType::Device.deterministic(&f.ieee_address))
                    .collect()
            },
            |devices| {
                devices
                    .iter()
                    .filter_map(|ieee| ieee.parse::<z2m::api::IeeeAddress>().ok())
                    .map(|ieee| RType::Device.deterministic(&ieee))
                    .collect()
            },
        );

        let mut res = self.state.lock().await;

        let mut scenes_new = HashSet::new();

        for scn in &grp.scenes {
            let scene = Scene {
                actions: vec![],
                auto_dynamic: false,
                group: link_room,
                metadata: SceneMetadata {
                    appdata: None,
                    image: guess_scene_icon(&scn.name),
                    name: scn.name.to_string(),
                },
                palette: json!({
                    "color": [],
                    "dimming": [],
                    "color_temperature": [],
                    "effects": [],
                }),
                speed: 0.5,
                recall: SceneRecall {
                    action: None,
                    dimming: None,
                    duration: None,
                },
                status: Some(SceneStatus {
                    active: SceneActive::Inactive,
                    last_recall: None,
                }),
            };

            let link_scene = RType::Scene.deterministic((link_room.rid, scn.id));

            res.aux_set(
                &link_scene,
                AuxData::new().with_topic(&topic).with_index(scn.id),
            );

            scenes_new.insert(link_scene.rid);
            res.add(&link_scene, Resource::Scene(scene))?;
        }

        if let Ok(room) = res.get::<Room>(&link_room) {
            log::info!(
                "[{}] {link_room:?} ({}) known, updating..",
                self.name,
                room.metadata.name
            );

            let scenes_old: HashSet<Uuid> =
                HashSet::from_iter(res.get_scenes_for_room(&link_room.rid));

            log::trace!("[{}] old scenes: {scenes_old:?}", self.name);
            log::trace!("[{}] new scenes: {scenes_new:?}", self.name);
            let gone = scenes_old.difference(&scenes_new);
            log::trace!("[{}]   deleted: {gone:?}", self.name);
            for uuid in gone {
                log::debug!(
                    "[{}] Deleting orphaned {uuid:?} in {link_room:?}",
                    self.name
                );
                let _ = res.delete(&RType::Scene.link_to(*uuid));
            }
        } else {
            log::debug!(
                "[{}] {link_room:?} ({}) is new, adding..",
                self.name,
                room_name
            );
        }

        let mut metadata = RoomMetadata::new(RoomArchetype::Home, room_name);
        if let Some(room_conf) = &room_conf {
            if let Some(name) = &room_conf.name {
                metadata.name = name.to_string();
            }
            if let Some(icon) = &room_conf.icon {
                metadata.archetype = *icon;
            }
        }

        let room = Room {
            children,
            metadata,
            services: btreeset![link_glight],
        };

        self.map.insert(topic.clone(), link_glight);
        self.rmap.insert(link_glight, topic.clone());
        self.rmap.insert(link_room, topic.clone());

        for id in &res.get_resource_ids_by_type(RType::BridgeHome) {
            res.update(id, |bh: &mut BridgeHome| {
                bh.children.insert(link_room);
            })?;
        }

        // the admin API names a room by its group topic, which is the key
        // `rooms:` uses
        res.aux_set(&link_room, AuxData::new().with_topic(&topic));

        // a room that is already known keeps its id, so a rename or a
        // changed membership is an update; adding it again would be a no-op
        // and leave the old name in place
        if res.get::<Room>(&link_room).is_ok() {
            res.update(&link_room.rid, |known: &mut Room| {
                known.metadata.clone_from(&room.metadata);
                known.children.clone_from(&room.children);
                known.services.clone_from(&room.services);
            })?;
        } else {
            res.add(&link_room, Resource::Room(room))?;
        }

        let glight = GroupedLight::new(link_room);

        res.add(&link_glight, Resource::GroupedLight(glight))?;
        drop(res);

        Ok(())
    }

    /// Place the devices the configuration puts in a room by hand.
    ///
    /// Membership normally comes from the z2m group set, which is what the
    /// Rooms screen writes. `devices.<ieee>.room` exists for a configuration
    /// written without the UI, and is applied once the groups are known so
    /// the room it names already exists.
    // the house error type is what every resource operation returns, so it
    // is propagated rather than flattened here
    #[allow(clippy::result_large_err)]
    pub async fn apply_device_rooms(&mut self) -> ApiResult<()> {
        // the key may be spelled `0x…` or, unquoted, as a YAML integer;
        // both are normalised to what zigbee2mqtt prints
        let config = self.store.active();
        let overrides: Vec<(String, String)> = config
            .devices
            .iter()
            .filter_map(|(key, device)| {
                let room = device.room.clone()?;
                let number = crate::config::ieee_number(key)?;
                Some((format!("0x{number:016x}"), room))
            })
            .collect();

        if overrides.is_empty() {
            return Ok(());
        }

        let mut res = self.state.lock().await;

        for (ieee, topic) in overrides {
            let Some(address) = self
                .network
                .values()
                .find(|dev| dev.ieee_address.to_string() == ieee)
                .map(|dev| dev.ieee_address.clone())
            else {
                log::debug!(
                    "[{}] No device at {ieee}, its room is not applied",
                    self.name
                );
                continue;
            };

            let Some(grouped) = self.map.get(&topic).copied() else {
                log::warn!(
                    "[{}] Device {ieee} names the unknown room {topic}",
                    self.name
                );
                continue;
            };

            let room_link = res.get::<GroupedLight>(&grouped)?.owner;
            let device_link = RType::Device.deterministic(&address);
            res.update(&room_link.rid, |room: &mut Room| {
                room.children.insert(device_link);
            })?;
        }
        drop(res);

        Ok(())
    }
}

/// The resource a sensor owns, before it has reported anything.
///
/// A sensor that has said nothing yet is still a sensor: the resource exists
/// with `*_valid: false` so a client shows it as unknown rather than absent.
fn initial_resource(kind: SensorKind, owner: ResourceLink) -> Resource {
    match kind {
        SensorKind::Motion => Resource::Motion(Motion {
            enabled: true,
            owner,
            motion: json!({
                "motion": false,
                "motion_valid": false,
            }),
            sensitivity: json!({
                "sensitivity": 0,
                "sensitivity_max": 4,
            }),
        }),
        SensorKind::Temperature => Resource::Temperature(Temperature {
            enabled: true,
            owner,
            temperature: json!({
                "temperature": 0.0,
                "temperature_valid": false,
            }),
        }),
        SensorKind::Illuminance => Resource::LightLevel(LightLevel {
            enabled: true,
            owner,
            light: json!({
                "light_level": 0,
                "light_level_valid": false,
            }),
        }),
        SensorKind::Contact => Resource::Contact(Contact(json!({
            "owner": owner,
            "contact": {
                "state": "no_contact",
                "state_valid": false,
            },
        }))),
        SensorKind::Remote => Resource::Button(Button {
            owner,
            metadata: ButtonMetadata { control_id: 0 },
            button: ButtonData {
                last_event: None,
                button_report: None,
                repeat_interval: Some(100),
                event_values: Some(json!(["initial_press", "repeat"])),
            },
        }),
    }
}

#[allow(clippy::match_same_arms)]
fn guess_scene_icon(name: &str) -> Option<ResourceLink> {
    let icon = match name {
        /* Built-in names */
        "Bright" => scene_icons::BRIGHT,
        "Relax" => scene_icons::RELAX,
        "Night Light" => scene_icons::NIGHT_LIGHT,
        "Rest" => scene_icons::REST,
        "Concentrate" => scene_icons::CONCENTRATE,
        "Dimmed" => scene_icons::DIMMED,
        "Energize" => scene_icons::ENERGIZE,
        "Read" => scene_icons::READ,
        "Cool Bright" => scene_icons::COOL_BRIGHT,

        /* Aliases */
        "Night" => scene_icons::NIGHT_LIGHT,
        "Cool" => scene_icons::COOL_BRIGHT,
        "Dim" => scene_icons::DIMMED,

        _ => return None,
    };

    Some(ResourceLink {
        rid: icon,
        rtype: RType::PublicImage,
    })
}
