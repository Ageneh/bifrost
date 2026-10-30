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

    /// The area names the entertainment configuration, so every command that
    /// follows carries it: two areas stream at once and a backend must route
    /// each one to its own state.
    EntertainmentStart(Uuid),
    EntertainmentFrame(Uuid, HueStreamLightsV2),
    EntertainmentStop(Uuid),

    ZigbeeDeviceDiscovery(ResourceLink, ZigbeeDeviceDiscoveryUpdate),
}
