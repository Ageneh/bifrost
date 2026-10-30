use serde::{Deserialize, Serialize};
use uuid::Uuid;

use hue::api::{
    GroupedLightUpdate, LightUpdate, ResourceLink, RoomUpdate, Scene, SceneUpdate,
    ZigbeeDeviceDiscoveryUpdate,
};
use hue::stream::HueStreamLightsV2;

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BackendRequest {
    LightUpdate(ResourceLink, LightUpdate),

    SceneCreate(ResourceLink, u32, Scene),
    SceneUpdate(ResourceLink, SceneUpdate),

    GroupedLightUpdate(ResourceLink, GroupedLightUpdate),

    RoomUpdate(ResourceLink, RoomUpdate),

    Delete(ResourceLink),

    EntertainmentStart(Uuid),
    EntertainmentFrame(HueStreamLightsV2),
    EntertainmentStop(),

    ZigbeeDeviceDiscovery(ResourceLink, ZigbeeDeviceDiscoveryUpdate),
}
