use hue::error::HueError;
use serde_json::Value;
use uuid::{Uuid, uuid};

use hue::api::{
    Bridge, Device, Entertainment, EntertainmentConfiguration, EntertainmentConfigurationAction,
    EntertainmentConfigurationChannels, EntertainmentConfigurationLocations,
    EntertainmentConfigurationNew, EntertainmentConfigurationServiceLocations,
    EntertainmentConfigurationStatus, EntertainmentConfigurationStreamMembers,
    EntertainmentConfigurationStreamProxy, EntertainmentConfigurationStreamProxyMode,
    EntertainmentConfigurationStreamProxyUpdate, EntertainmentConfigurationUpdate, Light,
    LightMode, Position, RType, Resource, ResourceLink,
};

use crate::error::ApiResult;
use crate::resource::Resources;
use crate::routes::auth::STANDARD_APPLICATION_ID;
use crate::routes::clip::{ApiV2Result, V2Reply};
use crate::server::appstate::AppState;

/// The v1 layout: fixed values fitting for an LCX005 gradient light chain.
///
/// The v1 `/api` path has no way to say where a light sits, so it cycles this
/// list. A v2 zone is laid out by the editor instead; see [`auto_layout`] for
/// the default it starts from.
pub const POSITIONS: &[Position] = &[
    Position {
        x: -0.4,
        y: 0.8,
        z: -0.4,
    },
    Position {
        x: -0.4,
        y: 0.8,
        z: 0.4,
    },
    Position {
        x: -0.22,
        y: 0.8,
        z: 0.4,
    },
    Position {
        x: 0.0,
        y: 0.8,
        z: 0.4,
    },
    Position {
        x: 0.22,
        y: 0.8,
        z: 0.4,
    },
    Position {
        x: 0.4,
        y: 0.8,
        z: 0.4,
    },
    Position {
        x: 0.4,
        y: 0.8,
        z: -0.4,
    },
];

pub async fn post_resource(state: &AppState, req: Value) -> ApiV2Result {
    let new: EntertainmentConfigurationNew = serde_json::from_value(req)?;

    let mut lock = state.res.lock().await;

    let locations = EntertainmentConfigurationLocations {
        service_locations: new
            .locations
            .service_locations
            .into_iter()
            .map(Into::into)
            .collect(),
    };

    let channels = make_channels(&lock, &locations.service_locations)?;
    let light_services = make_services(&lock, &locations.service_locations)?;

    let auto_node = find_bridge_entertainment(&lock)?;

    let obj = Resource::EntertainmentConfiguration(EntertainmentConfiguration {
        name: new.metadata.name.clone(),
        configuration_type: new.configuration_type,
        metadata: new.metadata,
        status: EntertainmentConfigurationStatus::Inactive,
        stream_proxy: new.stream_proxy.map_or_else(
            || EntertainmentConfigurationStreamProxy {
                mode: EntertainmentConfigurationStreamProxyMode::Auto,
                node: auto_node,
            },
            |sp| match sp {
                EntertainmentConfigurationStreamProxyUpdate::Auto => {
                    EntertainmentConfigurationStreamProxy {
                        mode: EntertainmentConfigurationStreamProxyMode::Auto,
                        node: auto_node,
                    }
                }
                EntertainmentConfigurationStreamProxyUpdate::Manual { node } => {
                    EntertainmentConfigurationStreamProxy {
                        mode: EntertainmentConfigurationStreamProxyMode::Manual,
                        node,
                    }
                }
            },
        ),
        channels,
        locations,
        light_services,
        active_streamer: None,
    });

    let rlink = ResourceLink::new(Uuid::new_v4(), obj.rtype());
    lock.add(&rlink, obj)?;
    drop(lock);

    V2Reply::ok(rlink)
}

/// The layout a zone starts from when the editor supplies no positions.
///
/// `segments` points spread evenly along the top edge of a screen, left to
/// right, so a two-light TV and a seven-segment gradient chain both start
/// somewhere sensible instead of on top of each other. The result depends on
/// nothing but `segments`, which is what lets the editor show the same
/// default the server would compute before anything is saved.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
pub fn auto_layout(segments: usize) -> Vec<Position> {
    /// The horizontal half-width of a screen, in the Hue coordinate system.
    const EDGE: f64 = 0.4;
    /// The top edge of a screen.
    const TOP: f64 = 0.8;
    /// Just in front of the screen's surface.
    const DEPTH: f64 = 0.4;

    /// Three decimals is what the Hue app shows and what round-trips exactly.
    fn round(value: f64) -> f64 {
        (value * 1000.0).round() / 1000.0
    }

    match segments {
        0 => vec![],
        1 => vec![Position::new(round(0.0), TOP, DEPTH)],
        count => {
            let step = (2.0 * EDGE) / (count - 1) as f64;
            (0..count)
                .map(|index| Position::new(round(-EDGE + step * index as f64), TOP, DEPTH))
                .collect()
        }
    }
}

/// How many channels one service location contributes: one per segment, or
/// one for a light that cannot address segments.
fn segment_count(ent: &Entertainment) -> usize {
    ent.segments
        .as_ref()
        .map_or(1, |segments| segments.segments.len().max(1))
}

/// Build the channel list from the positions the editor supplied.
///
/// A channel sits where its segment was placed. A location that names fewer
/// positions than it has segments falls back to [`auto_layout`] for the rest,
/// rather than collapsing the segments onto one point: a gradient chain whose
/// seven segments all sat in the same place would render one colour.
fn make_channels(
    lock: &Resources,
    locations: &[EntertainmentConfigurationServiceLocations],
) -> ApiResult<Vec<EntertainmentConfigurationChannels>> {
    let mut channels: Vec<EntertainmentConfigurationChannels> = vec![];

    let mut channel_id = 0;
    for location in locations {
        let ent: &Entertainment = lock.get(&location.service)?;
        let count = segment_count(ent);
        let fallback = auto_layout(count);

        for index in 0..count {
            let position = location
                .positions
                .get(index)
                .or_else(|| fallback.get(index))
                .cloned()
                .unwrap_or_default();

            channels.push(EntertainmentConfigurationChannels {
                channel_id,
                position,
                members: vec![EntertainmentConfigurationStreamMembers {
                    service: location.service,
                    index: u16::try_from(index)?,
                }],
            });
            channel_id += 1;
        }
    }

    Ok(channels)
}

fn make_services(
    lock: &Resources,
    locations: &[EntertainmentConfigurationServiceLocations],
) -> ApiResult<Vec<ResourceLink>> {
    let mut res = vec![];

    for location in locations {
        let ent: &Entertainment = lock.get(&location.service)?;
        if let Some(ren) = ent.renderer_reference {
            res.push(ren);
        }
    }

    Ok(res)
}

fn find_bridge_entertainment(lock: &Resources) -> ApiResult<ResourceLink> {
    let bridge_id = lock.get_resource_ids_by_type(RType::Bridge)[0];

    let bridge: &Bridge = lock.get_id(bridge_id)?;

    let bridge_dev: &Device = lock.get_id(bridge.owner.rid)?;

    let bridge_ent = bridge_dev
        .services
        .iter()
        .find(|obj| obj.rtype == RType::Entertainment)
        .copied()
        .ok_or(HueError::NotFound(bridge_id))?;

    Ok(bridge_ent)
}

pub async fn put_resource_id(state: &AppState, rlink: ResourceLink, put: Value) -> ApiV2Result {
    let upd: EntertainmentConfigurationUpdate = serde_json::from_value(put)?;

    let mut lock = state.res.lock().await;

    let mut locations = None;
    let mut channels = vec![];
    let mut light_services = vec![];

    if let Some(locs) = &upd.locations {
        let newlocs = EntertainmentConfigurationLocations {
            service_locations: locs
                .service_locations
                .clone()
                .into_iter()
                .map(Into::into)
                .collect(),
        };
        channels = make_channels(&lock, &newlocs.service_locations)?;
        light_services = make_services(&lock, &newlocs.service_locations)?;
        locations = Some(newlocs);
    }

    if let Some(action) = &upd.action {
        let ent: &EntertainmentConfiguration = lock.get(&rlink)?;
        // every light in the zone, not the first one: an area is what the
        // owner pointed at, and a zone with no lights yet simply has none
        // to move
        let svc = ent.light_services.clone();

        for link in svc {
            lock.update::<Light>(&link.rid, |light| {
                light.mode = match action {
                    EntertainmentConfigurationAction::Start => LightMode::Streaming,
                    EntertainmentConfigurationAction::Stop => LightMode::Normal,
                }
            })?;
        }
    }

    let bridge_ent = find_bridge_entertainment(&lock)?;

    lock.update::<EntertainmentConfiguration>(&rlink.rid, |ec| {
        if let Some(_locations) = upd.locations {
            ec.locations = locations.unwrap();
            ec.channels = channels;
            ec.light_services = light_services;
        }

        if let Some(proxy) = upd.stream_proxy {
            match proxy {
                EntertainmentConfigurationStreamProxyUpdate::Auto => {
                    ec.stream_proxy.mode = EntertainmentConfigurationStreamProxyMode::Auto;
                    ec.stream_proxy.node = bridge_ent;
                }
                EntertainmentConfigurationStreamProxyUpdate::Manual { node } => {
                    ec.stream_proxy.mode = EntertainmentConfigurationStreamProxyMode::Manual;
                    ec.stream_proxy.node = node;
                }
            }
        }

        if let Some(ctype) = upd.configuration_type {
            ec.configuration_type = ctype;
        }

        if let Some(md) = upd.metadata {
            ec.metadata = md;
        }

        if let Some(action) = upd.action {
            match action {
                EntertainmentConfigurationAction::Start => {
                    ec.active_streamer =
                        Some(RType::AuthV1.link_to(uuid!(STANDARD_APPLICATION_ID)));
                    ec.status = EntertainmentConfigurationStatus::Active;
                }
                EntertainmentConfigurationAction::Stop => {
                    ec.active_streamer = None;
                    ec.status = EntertainmentConfigurationStatus::Inactive;
                }
            }
        }
    })?;

    drop(lock);

    V2Reply::ok(rlink)
}
