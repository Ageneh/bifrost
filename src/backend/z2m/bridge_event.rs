use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use tokio_tungstenite::tungstenite;
use uuid::Uuid;

use hue::api::{DimmingUpdate, GroupedLight, Light, LightUpdate, RType, Resource, Room};
use z2m::api::{
    BridgeDevices, BridgeEvent, DeviceRemoveResponse, Group, GroupMemberChange, Message,
    RawMessage, Response,
};
use z2m::update::DeviceUpdate;

use crate::backend::z2m::Z2mBackend;
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

    async fn handle_update(&mut self, rid: &Uuid, payload: &Value) -> ApiResult<()> {
        if let Value::String(string) = payload {
            if string.is_empty() {
                log::debug!("Ignoring empty payload for {rid}");
                return Ok(());
            }
        }

        let upd = DeviceUpdate::deserialize(payload)?;

        let obj = self.state.lock().await.get_resource_by_id(rid)?.obj;
        match obj {
            Resource::Light(_) => {
                if let Err(e) = self.handle_update_light(rid, &upd).await {
                    log::error!("FAIL: {e:?} in {upd:?}");
                }
            }
            Resource::GroupedLight(_) => {
                if let Err(e) = self.handle_update_grouped_light(rid, &upd).await {
                    log::error!("FAIL: {e:?} in {upd:?}");
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn handle_device_message(&mut self, msg: RawMessage) -> ApiResult<()> {
        if let Some(name) = msg.topic.strip_suffix("/availability") {
            // https://www.zigbee2mqtt.io/guide/usage/mqtt_topics_and_messages.html#zigbee2mqtt-friendly-name-availability
            self.handle_availability(name, &msg.payload);
            return Ok(());
        }

        if msg.topic.ends_with("/action") {
            // action: https://www.home-assistant.io/integrations/device_trigger.mqtt/
            self.touch_device(&msg.topic);
            return Ok(());
        }

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
        let res = self.handle_update(&val.rid, &msg.payload).await;
        if let Err(ref err) = res {
            log::error!(
                "Cannot parse update: {err}\n{}",
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
            if config.device(&ieee).and_then(|device| device.enabled) == Some(false) {
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
            } else {
                log::debug!(
                    "[{}] Ignoring unsupported device {}",
                    self.name,
                    dev.friendly_name
                );
                self.ignore.insert(dev.friendly_name.clone());
            }

            self.publish_device(dev);
            /*
            if dev.expose_action() {
                log::info!(
                    "[{}] Adding switch {:?}: [{}] ({})",
                    self.name,
                    dev.ieee_address,
                    dev.friendly_name,
                    dev.model_id.as_deref().unwrap_or("<unknown model>")
                );
                self.add_switch(dev).await?;
            }
            */
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
