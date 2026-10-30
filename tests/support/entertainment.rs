//! Building the pieces a wave-7 entertainment test needs: an area over the
//! lights a mock announces, and the frames a Sync box would send.

#![allow(dead_code)]

use uuid::Uuid;

use bifrost::resource::Resources;
use hue::api::{
    Entertainment, EntertainmentConfiguration, EntertainmentConfigurationChannels,
    EntertainmentConfigurationLocations, EntertainmentConfigurationMetadata,
    EntertainmentConfigurationServiceLocations, EntertainmentConfigurationStatus,
    EntertainmentConfigurationStreamProxy, EntertainmentConfigurationStreamProxyMode,
    EntertainmentConfigurationType, Position, RType, Resource, ResourceLink,
};
use hue::stream::{HueStreamLightsV2, Rgb16, Rgb16V2};

/// Add an entertainment configuration over the entertainment services of
/// `lights`, each on its own channel.
///
/// The links come from the import, which derives them from the IEEE address;
/// they are looked up in the inventory rather than recomputed here, so the
/// test cannot drift from the way the backend builds them.
pub fn add_area(res: &mut Resources, name: &str, lights: &[ResourceLink]) -> Uuid {
    let mut channels = vec![];
    let mut light_services = vec![];

    for (index, link) in lights.iter().enumerate() {
        let ent: &Entertainment = res
            .get(link)
            .unwrap_or_else(|_| panic!("no entertainment service for {link:?}"));
        light_services.push(ent.renderer_reference.expect("no renderer reference"));

        channels.push(EntertainmentConfigurationChannels {
            channel_id: u32::try_from(index).unwrap(),
            position: Position::default(),
            members: vec![hue::api::EntertainmentConfigurationStreamMembers {
                service: *link,
                index: 0,
            }],
        });
    }

    let bridge_ent = res
        .get_resource_ids_by_type(RType::Bridge)
        .first()
        .map(|id| RType::Entertainment.deterministic(id.to_string()))
        .expect("no bridge");

    let area = Uuid::new_v4();
    let link = ResourceLink::new(area, RType::EntertainmentConfiguration);

    res.add(
        &link,
        Resource::EntertainmentConfiguration(EntertainmentConfiguration {
            name: name.to_string(),
            configuration_type: EntertainmentConfigurationType::Screen,
            metadata: EntertainmentConfigurationMetadata {
                name: name.to_string(),
            },
            status: EntertainmentConfigurationStatus::Inactive,
            stream_proxy: EntertainmentConfigurationStreamProxy {
                mode: EntertainmentConfigurationStreamProxyMode::Auto,
                node: bridge_ent,
            },
            locations: EntertainmentConfigurationLocations {
                service_locations: lights
                    .iter()
                    .map(|link| EntertainmentConfigurationServiceLocations {
                        equalization_factor: 1.0,
                        position: Position::default(),
                        positions: vec![Position::default()],
                        service: *link,
                    })
                    .collect(),
            },
            light_services,
            channels,
            active_streamer: None,
        }),
    )
    .expect("could not add the area");

    area
}

/// One frame of a single colour on `channels` channels, as a v2 Sync packet.
#[must_use]
pub fn frame(channels: u8) -> HueStreamLightsV2 {
    HueStreamLightsV2::Rgb(
        (0..channels)
            .map(|channel| Rgb16V2 {
                channel,
                rgb: Rgb16 {
                    r: 0xffff,
                    g: 0,
                    b: 0,
                },
            })
            .collect(),
    )
}
