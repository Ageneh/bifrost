use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite;
use uuid::Uuid;

use hue::api::{
    Button, ButtonReport, Contact, DevicePower, DimmingUpdate, GroupedLight, Light, LightLevel,
    LightUpdate, Motion, RType, Resource, Room, Temperature,
};
use z2m::api::{
    BridgeDevices, BridgeEvent, DeviceRemoveResponse, Group, GroupMemberChange, Message,
    RawMessage, Response,
};
use z2m::update::DeviceUpdate;

use crate::backend::z2m::Z2mBackend;
use crate::backend::z2m::sensor::{self, SensorKind};
use crate::error::{ApiError, ApiResult};
use crate::inventory::{Availability, GroupRef, PermitJoinState, RoomRef, from_z2m};

impl Z2mBackend {
    /// Publish one device into the inventory, keeping what only it knows.
    fn publish_device(&self, dev: &z2m::api::Device) {
        let Some(inventory) = self.inventory() else {
            return;
        };
        let ieee = dev.ieee_address.to_string();
        let previous = inventory.device(&self.name, &ieee);
        let mut info = from_z2m(&self.name, dev, previous.as_ref());
        info.group = self.group_of.get(&ieee).cloned();
        info.imported = self.map.contains_key(&dev.friendly_name);

        // a device Bifrost could serve but does not is not silent: the
        // reason names the switch that changes it
        if info.supported && !info.imported {
            let enabled = self
                .store
                .active()
                .device(&ieee)
                .and_then(|device| device.enabled);
            info.unsupported_reason = Some(if enabled == Some(false) {
                format!("Disabled in the configuration: devices.{ieee}.enabled is false")
            } else {
                format!("Not imported: set devices.{ieee}.enabled to serve it over the Hue API")
            });
        }

        inventory.upsert(info);
    }

    /// Recompute which room each of this server's devices is in.
    ///
    /// `Resources` is taken first and released before the inventory lock is
    /// taken, which is the only order the two are ever locked in.
    async fn sync_placement(&self) {
        let Some(inventory) = self.inventory() else {
            return;
        };

        let placements: Vec<(String, Option<RoomRef>)> = {
            let lock = self.state.lock().await;
            self.network
                .values()
                .map(|dev| {
                    let link = RType::Device.deterministic(&dev.ieee_address);
                    let room = lock
                        .get_resource_ids_by_type(RType::Room)
                        .into_iter()
                        .find_map(|rid| {
                            let room = lock.get_id::<Room>(rid).ok()?;
                            room.children.contains(&link).then(|| RoomRef {
                                rid: rid.to_string(),
                                name: room.metadata.name.clone(),
                            })
                        });
                    (dev.ieee_address.to_string(), room)
                })
                .collect()
        };

        for (ieee, room) in placements {
            let group = self.group_of.get(&ieee).cloned();
            inventory.set_placement(&self.name, &ieee, room, group);
        }
    }

    /// The IEEE address of a device, by its friendly name.
    fn device_ieee(&self, friendly_name: &str) -> Option<String> {
        self.network
            .get(friendly_name)
            .map(|dev| dev.ieee_address.to_string())
    }

    /// Apply a `<name>/availability` report.
    fn handle_availability(&self, friendly_name: &str, payload: &Value) {
        let (Some(inventory), Some(ieee)) = (self.inventory(), self.device_ieee(friendly_name))
        else {
            return;
        };

        let availability = match payload.get("state").and_then(Value::as_str) {
            Some("online") => Availability::Online,
            Some("offline") => Availability::Offline,
            _ => return,
        };

        inventory.set_availability(&self.name, &ieee, availability, Some(Utc::now()));
    }

    /// Record that a device reported something on `topic`.
    fn touch_device(&self, topic: &str) {
        let (Some(inventory), Some(ieee)) = (self.inventory(), self.device_ieee(topic)) else {
            return;
        };
        inventory.touch(&self.name, &ieee, Utc::now());
    }

    /// Apply a `bridge/state` report: z2m itself came up or went down.
    fn handle_bridge_state(&self, payload: &Value) {
        let Some(inventory) = self.inventory() else {
            return;
        };

        match payload.get("state").and_then(Value::as_str) {
            Some("online") => inventory.server_up(&self.name),
            Some("offline") => inventory.server_down(&self.name),
            _ => {}
        }
    }

    /// Apply a `bridge/permit_join` report.
    fn handle_permit_join(&self, payload: &Value) {
        let Some(inventory) = self.inventory() else {
            return;
        };

        let time = payload
            .get("time")
            .and_then(Value::as_u64)
            .and_then(|time| u32::try_from(time).ok())
            .unwrap_or_default();
        let device = payload
            .get("device")
            .and_then(Value::as_str)
            .map(str::to_string);

        inventory.set_permit_join(
            &self.name,
            PermitJoinState {
                time,
                device,
                opened_at: (time > 0).then(Utc::now),
            },
        );
    }

    /// Apply a `bridge/event`: the announcements z2m makes as devices come
    /// and go.
    fn handle_bridge_event_record(&mut self, event: &BridgeEvent) {
        if event.event_type != "device_removed" {
            return;
        }

        let Some(data) = event.data.as_object() else {
            return;
        };

        if let Some(name) = data.get("friendly_name").and_then(Value::as_str) {
            self.forget_device(name);
            return;
        }

        // a removal without a friendly name still names the address
        let Some(ieee) = data.get("ieee_address").and_then(Value::as_str) else {
            return;
        };
        let friendly_name = self
            .network
            .iter()
            .find(|(_, dev)| dev.ieee_address.to_string() == ieee)
            .map(|(name, _)| name.clone());
        if let Some(name) = friendly_name {
            self.forget_device(&name);
        }
    }

    /// Record the group membership `group` declares.
    fn record_group_members(&mut self, group: &Group, added: bool) {
        self.group_ids.insert(group.friendly_name.clone(), group.id);
        let reference = GroupRef {
            id: group.id,
            name: group.friendly_name.clone(),
        };
        for member in &group.members {
            let ieee = member.ieee_address.to_string();
            if added {
                self.group_of.insert(ieee, reference.clone());
            } else {
                self.group_of.remove(&ieee);
            }
        }
    }

    async fn handle_update_light(&mut self, uuid: &Uuid, devupd: &DeviceUpdate) -> ApiResult<()> {
        let upd: LightUpdate = devupd.into();

        let mut lock = self.state.lock().await;
        lock.update::<Light>(uuid, |light| *light += &upd)?;

        self.learner.learn(uuid, &lock, devupd)?;
        self.learner.collect(&mut lock)?;
        drop(lock);

        Ok(())
    }

    async fn handle_update_grouped_light(&self, uuid: &Uuid, upd: &DeviceUpdate) -> ApiResult<()> {
        let mut res = self.state.lock().await;
        res.update::<GroupedLight>(uuid, |glight| {
            if let Some(state) = &upd.state {
                glight.on = Some((*state).into());
            }

            if let Some(b) = upd.brightness {
                glight.dimming = Some(DimmingUpdate {
                    brightness: b / 254.0 * 100.0,
                });
            }
        })
    }

    // the house error type is what every resource operation returns, so it is
    // propagated rather than flattened here
    #[allow(clippy::result_large_err)]
    async fn handle_update(
        &mut self,
        topic: &str,
        rid: &Uuid,
        upd: &DeviceUpdate,
    ) -> ApiResult<()> {
        let obj = self.state.lock().await.get_resource_by_id(rid)?.obj;
        match obj {
            Resource::Light(_) => {
                if let Err(e) = self.handle_update_light(rid, upd).await {
                    log::error!("FAIL: {e:?} in {upd:?}");
                }
            }
            Resource::GroupedLight(_) => {
                if let Err(e) = self.handle_update_grouped_light(rid, upd).await {
                    log::error!("FAIL: {e:?} in {upd:?}");
                }
            }
            Resource::Motion(_)
            | Resource::Temperature(_)
            | Resource::LightLevel(_)
            | Resource::Contact(_) => {
                if let Err(e) = self.handle_update_sensor(topic, rid, upd).await {
                    log::error!("FAIL: {e:?} in {upd:?}");
                }
            }
            // a button changes on its `/action` topic, not on the device one
            _ => {}
        }

        if let Err(e) = self.handle_update_power(topic, upd).await {
            log::error!("FAIL: {e:?} in {upd:?}");
        }

        Ok(())
    }

    /// Apply one reading to the Hue resource its class owns.
    ///
    /// A binary is turned into "is it happening" first, because the z2m value
    /// means different things on different firmware; see
    /// [`crate::backend::z2m::sensor::is_active`].
    #[allow(clippy::result_large_err, clippy::significant_drop_tightening)]
    async fn handle_update_sensor(
        &self,
        topic: &str,
        rid: &Uuid,
        upd: &DeviceUpdate,
    ) -> ApiResult<()> {
        let device = self.network.get(topic);
        let mut res = self.state.lock().await;

        // every reading carries when it changed: v1 writes that timestamp
        // into `state.lastupdated`, and a client that shows "3 min ago"
        // needs the same one
        let changed = Utc::now();

        if let Some(raw) = upd.occupancy {
            let motion = device.map_or(raw, |device| sensor::is_active(device, "occupancy", raw));
            res.update::<Motion>(rid, |known| {
                known.motion = json!({
                    "motion": motion,
                    "motion_valid": true,
                    "motion_report": { "changed": changed, "motion": motion },
                });
            })?;
        }

        if let Some(temperature) = upd.temperature {
            res.update::<Temperature>(rid, |known| {
                known.temperature = json!({
                    "temperature": temperature,
                    "temperature_valid": true,
                    "temperature_report": { "changed": changed, "temperature": temperature },
                });
            })?;
        }

        if let Some(lux) = upd.illuminance {
            let level = sensor::light_level(lux);
            res.update::<LightLevel>(rid, |known| {
                known.light = json!({
                    "light_level": level,
                    "light_level_valid": true,
                    "light_level_report": { "changed": changed, "light_level": level },
                });
            })?;
        }

        if let Some(raw) = upd.contact {
            let open = device.map_or(raw, |device| sensor::is_active(device, "contact", raw));
            let state = if open { "contact" } else { "no_contact" };
            res.update::<Contact>(rid, |known| {
                known.0["contact"] = json!({
                    "state": state,
                    "state_valid": true,
                    "contact_report": { "changed": changed, "state": state },
                });
            })?;
        }

        Ok(())
    }

    /// Mirror the battery level onto the device's `DevicePower` resource.
    #[allow(clippy::result_large_err, clippy::significant_drop_tightening)]
    async fn handle_update_power(&self, topic: &str, upd: &DeviceUpdate) -> ApiResult<()> {
        let Some(level) = upd.battery else {
            return Ok(());
        };
        let Some(device) = self.network.get(topic) else {
            return Ok(());
        };

        let link = RType::DevicePower.deterministic(&device.ieee_address);
        let mut res = self.state.lock().await;
        if res.get::<DevicePower>(&link).is_err() {
            return Ok(());
        }
        let power_state = sensor::power_state(Some(sensor::battery_percent(level)));
        res.update::<DevicePower>(&link.rid, |known| {
            known.power_state = power_state;
        })?;

        Ok(())
    }

    /// Record every sensor reading a device message carries.
    ///
    /// This runs for a sensor that is not imported into the Hue API as well:
    /// the Devices screen shows the reading either way, and an opt-in sensor
    /// that has not been opted in must still report.
    fn record_readings(&self, topic: &str, upd: &DeviceUpdate) {
        let (Some(inventory), Some(ieee)) = (self.inventory(), self.device_ieee(topic)) else {
            return;
        };

        let now = Utc::now();

        if let Some(level) = upd.battery {
            inventory.set_battery(&self.name, &ieee, sensor::battery_percent(level));
        }

        let (Some(device), Some(kind)) = (self.network.get(topic), self.sensors.get(topic)) else {
            return;
        };

        // the reading is stored with the same meaning the resource carries,
        // so the screen and the Hue app never disagree
        let value = match *kind {
            SensorKind::Motion => upd
                .occupancy
                .map(|raw| json!(sensor::is_active(device, "occupancy", raw))),
            SensorKind::Temperature => upd.temperature.map(|value| json!(value)),
            SensorKind::Illuminance => upd.illuminance.map(|value| json!(value)),
            SensorKind::Contact => upd
                .contact
                .map(|raw| json!(sensor::is_active(device, "contact", raw))),
            SensorKind::Remote => None,
        };

        if let Some(value) = value {
            inventory.set_reading(&self.name, &ieee, kind.property(), value, now);
        }
    }

    /// Apply a `<name>/action` report: a remote's button press.
    ///
    /// The press is both a reading the Devices screen shows and an update of
    /// the remote's `Button` resource, so the Hue app and the console see the
    /// same thing at the same moment.
    async fn handle_action(&self, name: &str, payload: &Value) {
        let Some(action) = payload.get("action").and_then(Value::as_str) else {
            return;
        };
        let action = action.to_string();

        self.touch_device(name);

        if let (Some(inventory), Some(ieee)) = (self.inventory(), self.device_ieee(name)) {
            inventory.set_reading(&self.name, &ieee, "action", json!(action), Utc::now());
        }

        if let Some(battery) = payload.get("battery").and_then(Value::as_f64) {
            if let Some(ieee) = self.device_ieee(name) {
                if let Some(inventory) = self.inventory() {
                    inventory.set_battery(&self.name, &ieee, sensor::battery_percent(battery));
                }
            }
        }

        let Some(link) = self.map.get(name).copied() else {
            return;
        };
        if link.rtype != RType::Button {
            return;
        }

        let report = ButtonReport {
            updated: Utc::now(),
            event: action.clone(),
        };
        let mut res = self.state.lock().await;
        if let Err(err) = res.update::<Button>(&link.rid, |button| {
            button.button.last_event = Some(json!({
                "event": action,
                "updated": report.updated,
            }));
            button.button.button_report = Some(report.clone());
        }) {
            log::error!("[{}] Cannot record action on {name}: {err:?}", self.name);
        }
    }

    async fn handle_device_message(&mut self, msg: RawMessage) -> ApiResult<()> {
        if let Some(name) = msg.topic.strip_suffix("/availability") {
            // https://www.zigbee2mqtt.io/guide/usage/mqtt_topics_and_messages.html#zigbee2mqtt-friendly-name-availability
            self.handle_availability(name, &msg.payload);
            return Ok(());
        }

        if let Some(name) = msg.topic.strip_suffix("/action") {
            // action: https://www.home-assistant.io/integrations/device_trigger.mqtt/
            self.handle_action(name, &msg.payload).await;
            return Ok(());
        }

        if let Value::String(string) = &msg.payload {
            if string.is_empty() {
                log::debug!("Ignoring empty payload on {}", msg.topic);
                return Ok(());
            }
        }

        let upd = match DeviceUpdate::deserialize(&msg.payload) {
            Ok(upd) => upd,
            Err(err) => {
                log::error!(
                    "Cannot parse update on {}: {err}\n{}",
                    msg.topic,
                    serde_json::to_string_pretty(&msg.payload)?
                );
                return Ok(());
            }
        };

        self.record_readings(&msg.topic, &upd);

        let Some(ref val) = self.map.get(&msg.topic).copied() else {
            if !self.ignore.contains(&msg.topic) {
                log::warn!(
                    "[{}] Notification on unknown topic {}",
                    self.name,
                    &msg.topic
                );
            }
            return Ok(());
        };

        self.touch_device(&msg.topic);
        if let Err(ref err) = self.handle_update(&msg.topic, &val.rid, &upd).await {
            log::error!(
                "Cannot apply update on {}: {err}\n{}",
                msg.topic,
                serde_json::to_string_pretty(&msg.payload)?
            );
        }

        /* return Ok here, since we do not want to break the event loop */
        Ok(())
    }

    async fn bridge_devices(&mut self, devices: &BridgeDevices) -> ApiResult<()> {
        // the overrides decide which devices reach the Hue API at all, so
        // they are read before anything is imported
        let config = self.store.active();

        for dev in devices {
            self.network.insert(dev.friendly_name.clone(), dev.clone());

            let ieee = dev.ieee_address.to_string();
            let enabled = config.device(&ieee).and_then(|device| device.enabled);

            // a sensor's class is remembered whether or not it is imported:
            // the Devices screen shows its reading either way
            if let Some(kind) = SensorKind::of(dev) {
                self.sensors.insert(dev.friendly_name.clone(), kind);
            }

            if enabled == Some(false) {
                log::info!(
                    "[{}] Leaving {ieee} ({}) out: disabled in the configuration",
                    self.name,
                    dev.friendly_name
                );
                self.ignore.insert(dev.friendly_name.clone());
                self.publish_device(dev);
                continue;
            }

            if let Some(exp) = dev.expose_light() {
                log::info!(
                    "[{}] Adding light {:?}: [{}] ({})",
                    self.name,
                    dev.ieee_address,
                    dev.friendly_name,
                    dev.model_id.as_deref().unwrap_or("<unknown model>")
                );
                self.add_light(dev, exp).await?;
            } else if let Some(kind) = self.sensors.get(&dev.friendly_name).copied() {
                // product F3: a sensor is opt-in on an install that predates
                // this wave. Importing every sensor on upgrade would add a
                // duplicate Hue resource for each one, so a sensor reaches
                // the Hue API only when the configuration names it enabled.
                if !enabled.unwrap_or_default() {
                    log::info!(
                        "[{}] Leaving {ieee} ({}) out: sensors are opt-in \
                         (set devices.{ieee}.enabled to import it)",
                        self.name,
                        dev.friendly_name
                    );
                    self.ignore.insert(dev.friendly_name.clone());
                } else {
                    log::info!(
                        "[{}] Adding {kind:?} {:?}: [{}] ({})",
                        self.name,
                        dev.ieee_address,
                        dev.friendly_name,
                        dev.model_id.as_deref().unwrap_or("<unknown model>")
                    );
                    self.add_sensor(dev, kind).await?;
                }
            } else {
                log::debug!(
                    "[{}] Ignoring unsupported device {}",
                    self.name,
                    dev.friendly_name
                );
                self.ignore.insert(dev.friendly_name.clone());
            }

            self.publish_device(dev);
        }

        // devices and groups arrive in either order, so the room overrides
        // are tried after each
        self.apply_device_rooms().await?;
        self.sync_placement().await;

        Ok(())
    }

    async fn bridge_device_remove(&mut self, data: &DeviceRemoveResponse) -> ApiResult<()> {
        if let Some(rlink) = self.map.get(&data.id) {
            match rlink.rtype {
                RType::Light => {
                    let mut lock = self.state.lock().await;
                    let owner = lock.get::<Light>(rlink)?.owner;
                    log::info!("Removing device: {owner:?}");
                    lock.delete(&owner)?;
                }
                rtype => {
                    log::warn!("Cannot handle removing resource of type {rtype:?}");
                }
            }
        }

        if let Some(_rlink) = self.map.remove(&data.id) {
            self.rmap.retain(|_, v| *v != data.id);
        }

        self.forget_device(&data.id);

        Ok(())
    }

    /// Drop a device from everything that tracks it, by friendly name.
    fn forget_device(&mut self, friendly_name: &str) {
        self.ignore.remove(friendly_name);
        self.sensors.remove(friendly_name);
        let Some(dev) = self.network.remove(friendly_name) else {
            return;
        };

        let ieee = dev.ieee_address.to_string();
        self.group_of.remove(&ieee);
        if let Some(inventory) = self.inventory() {
            inventory.remove(&self.name, &ieee);
        }
    }

    #[allow(clippy::collapsible_else_if)]
    async fn bridge_group_member_change(
        &mut self,
        change: &GroupMemberChange,
        added: bool,
    ) -> ApiResult<()> {
        if let Some(light) = self.map.get(&change.device).copied() {
            let mut lock = self.state.lock().await;
            let device = lock.get::<Light>(&light)?.clone();

            let device_link = device.owner;
            if let Some(room) = self.map.get(&change.group) {
                let room_link = lock.get::<GroupedLight>(room)?.owner;
                let exists = lock
                    .get::<Room>(&room_link)?
                    .children
                    .contains(&device_link);

                if added {
                    if !exists {
                        lock.update(&room_link.rid, |room: &mut Room| {
                            room.children.insert(device_link);
                        })?;
                    }
                } else {
                    if exists {
                        lock.update(&room_link.rid, |room: &mut Room| {
                            room.children.remove(&device_link);
                        })?;
                    }
                }
            }
        }

        // the membership is what the inventory's group hop reads
        if let Some(ieee) = self.device_ieee(&change.device) {
            if added {
                if let Some(id) = self.group_ids.get(&change.group).copied() {
                    self.group_of.insert(
                        ieee,
                        GroupRef {
                            id,
                            name: change.group.clone(),
                        },
                    );
                }
            } else {
                self.group_of.remove(&ieee);
            }
        }

        self.sync_placement().await;

        Ok(())
    }

    async fn handle_bridge_message(&mut self, msg: Message) -> ApiResult<()> {
        #[allow(unused_variables)]
        match &msg {
            Message::BridgeInfo(obj) => { /* println!("{obj:#?}"); */ }
            Message::BridgeLogging(obj) => { /* println!("{obj:#?}"); */ }
            Message::BridgeExtensions(obj) => { /* println!("{obj:#?}"); */ }
            Message::BridgeEvent(obj) => {
                self.handle_bridge_event_record(obj);
            }
            Message::BridgeDefinitions(obj) => { /* println!("{obj:#?}"); */ }
            Message::BridgeState(obj) => {
                self.handle_bridge_state(obj);
            }
            Message::BridgeConverters(obj) => { /* println!("{obj:#?}"); */ }
            Message::BridgeOptions(obj) => { /* println!("{obj:#?}"); */ }
            Message::BridgePermitJoin(obj) => {
                self.handle_permit_join(obj);
            }
            Message::BridgeTouchlinkScan(obj) => {}
            Message::BridgeDeviceOptions(obj) => {}
            Message::BridgeNetworkmap(obj) => {}
            Message::BridgeDeviceOtaUpdateCheck(obj) => {}
            Message::BridgeDeviceConfigureReporting(obj) => {}
            Message::BridgeConfig(obj) => {}
            Message::BridgeResponseGroupAdd(obj) => {}
            Message::BridgeResponseGroupRemove(obj) => {}
            Message::BridgeResponseGroupRename(obj) => {}
            Message::BridgeResponseGroupOptions(obj) => {}

            Message::BridgeDevices(obj) => {
                self.bridge_devices(obj).await?;
            }

            Message::BridgeGroups(obj) => {
                /* println!("{obj:#?}"); */
                for grp in obj {
                    self.record_group_members(grp, true);
                    self.add_group(grp).await?;
                }
                self.apply_device_rooms().await?;
                self.sync_placement().await;
            }

            Message::BridgeGroupMembersAdd(change) | Message::BridgeGroupMembersRemove(change) => {
                let Response::Ok { data: change, .. } = change else {
                    log::warn!("[{}] Error reported from z2m: {change:?}", self.name);
                    return Ok(());
                };

                let added = matches!(msg, Message::BridgeGroupMembersAdd(_));
                self.bridge_group_member_change(change, added).await?;
            }

            Message::BridgeDeviceRemove(obj) => {
                let Response::Ok { data, .. } = obj else {
                    log::warn!("[{}] Error reported from z2m: {obj:?}", self.name);
                    return Ok(());
                };

                self.bridge_device_remove(data).await?;
            }
        }
        Ok(())
    }

    pub async fn handle_bridge_event(&mut self, pkt: tungstenite::Message) -> ApiResult<()> {
        let tungstenite::Message::Text(txt) = pkt else {
            log::error!("[{}] Received non-text message on websocket :(", self.name);
            return Err(ApiError::UnexpectedZ2mReply(pkt));
        };

        let raw_msg = serde_json::from_str::<RawMessage>(&txt);

        log::trace!("[{}] Incoming z2m message: {txt}", self.name);

        let msg = raw_msg.map_err(|err| {
            log::error!(
                "[{}] Invalid websocket message: {:#?} [{}..]",
                self.name,
                err,
                &txt.chars().take(128).collect::<String>()
            );
            err
        })?;

        /* bridge messages are handled differently. everything else is a device message */
        if !msg.topic.starts_with("bridge/") {
            return self.handle_device_message(msg).await;
        }

        match serde_json::from_str(&txt) {
            Ok(bridge_msg) => self.handle_bridge_message(bridge_msg).await,
            Err(err) => {
                match msg.topic.as_str() {
                    topic @ ("bridge/devices" | "bridge/groups") => {
                        log::error!(
                            "[{}] Failed to parse critical z2m bridge message on [{}]:",
                            self.name,
                            topic,
                        );
                        log::error!("[{}] {}", self.name, serde_json::to_string(&msg.payload)?);
                        Err(err)?
                    }
                    topic => {
                        log::error!(
                            "[{}] Failed to parse (non-critical) z2m bridge message on [{}]:",
                            self.name,
                            topic
                        );
                        log::error!("{}", serde_json::to_string(&msg.payload)?);

                        /* Suppress this non-critical error, to avoid breaking the event loop */
                        Ok(())
                    }
                }
            }
        }
    }
}
