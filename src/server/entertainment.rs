use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::fd::AsFd;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use nix::sys::socket;
use nix::sys::socket::sockopt::RcvBuf;
use openssl::ssl::{Ssl, SslContext, SslMethod};
use tokio::io::AsyncReadExt;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_openssl::SslStream;
use udp_stream::{UdpListenBuilder, UdpListener, UdpStream};
use uuid::Uuid;

use bifrost_api::backend::BackendRequest;
use hue::api::{Device, EntertainmentConfiguration, Light, RType};
use hue::error::HueError;
use hue::stream::{
    HueStreamLightsV1, HueStreamLightsV2, HueStreamPacket, HueStreamPacketV1, HueStreamPacketV2,
    Rgb16V2, Xy16V2,
};
use svc::traits::Service;

use crate::config_store::ConfigStore;
use crate::error::{ApiError, ApiResult};
use crate::resource::Resources;
use crate::routes::auth::STANDARD_CLIENT_KEY;
use crate::server::streams::StreamRegistry;

pub struct EntertainmentService {
    addr: SocketAddr,
    udp: Option<Arc<UdpListener>>,
    ctx: Option<Arc<SslContext>>,
    res: Arc<Mutex<Resources>>,
    streams: Arc<StreamRegistry>,
    /// Read on every accept, so a changed `bifrost.entertainment.max_streams`
    /// applies without restarting the listener.
    store: Arc<ConfigStore>,
}

impl EntertainmentService {
    pub fn new(
        addr: Ipv4Addr,
        port: u16,
        res: Arc<Mutex<Resources>>,
        streams: Arc<StreamRegistry>,
        store: Arc<ConfigStore>,
    ) -> ApiResult<Self> {
        let res = Self {
            addr: SocketAddr::new(addr.into(), port),
            udp: None,
            ctx: None,
            res,
            streams,
            store,
        };

        Ok(res)
    }

    /// How many sessions may stream at once.
    fn max_streams(&self) -> usize {
        usize::try_from(self.store.active().bifrost.entertainment.max_streams)
            .unwrap_or(1)
            .max(1)
    }
}

/// One DTLS session: everything a stream needs, and nothing that outlives it.
///
/// A session runs on its own task, so a sender that panics, stalls or
/// disappears takes nothing else with it.
#[derive(Clone)]
pub struct Session {
    res: Arc<Mutex<Resources>>,
    streams: Arc<StreamRegistry>,
}

impl Session {
    async fn read_frame(sess: &mut SslStream<UdpStream>, buf: &mut [u8]) -> ApiResult<usize> {
        const TIMEOUT: Duration = Duration::from_secs(10);

        match timeout(TIMEOUT, sess.read(buf)).await {
            Ok(Err(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                log::debug!("Sync stream stopped by sender");
                Ok(0)
            }
            Ok(Err(err)) => {
                log::error!("Error while trying to read sync data: {err:?}");
                Err(ApiError::EntStreamDesync)
            }
            Err(_) => {
                log::warn!("Timeout while waiting for sync data");
                Err(ApiError::EntStreamTimeout)
            }
            Ok(Ok(n)) => {
                log::trace!("Read {n} bytes of sync data");
                Ok(n)
            }
        }
    }

    #[allow(clippy::unused_async, unreachable_code, unused_variables)]
    pub fn translate_v1_frame(
        res: &Resources,
        v1: HueStreamPacketV1,
    ) -> ApiResult<HueStreamPacketV2> {
        let mut lights = BTreeMap::new();
        for id_v1 in v1.light_ids() {
            let uuid = res.from_id_v1(id_v1)?;
            lights.insert(id_v1, uuid);
        }
        let light_ids: BTreeSet<_> = lights.values().copied().collect();

        let mut best_included = 0;
        let mut best_excluded = 0;
        let mut best_area = None;

        for uuid in res.get_resource_ids_by_type(RType::EntertainmentConfiguration) {
            let entconf = res.get_id::<EntertainmentConfiguration>(uuid)?;

            let ent_light_ids: BTreeSet<Uuid> =
                entconf.light_services.iter().map(|ls| ls.rid).collect();

            let included = light_ids.intersection(&ent_light_ids).count();
            let excluded = light_ids.difference(&ent_light_ids).count();

            if (included > best_included)
                || ((included == best_included) && (excluded < best_excluded))
            {
                log::trace!("Found candidate entertainment_configuration: {uuid}");
                log::trace!("  lights included: {included}");
                log::trace!("  lights excluded: {excluded}");
                best_included = included;
                best_excluded = excluded;
                best_area = Some(uuid);
            }
        }

        if let Some(area) = best_area {
            let mut mapping = BTreeMap::new();
            for (id_v1, id) in &lights {
                let light: &Light = res.get_id(*id)?;
                let dev: &Device = res.get(&light.owner)?;
                let ent_svc = dev.entertainment_service().ok_or(HueError::NotFound(*id))?;

                mapping.insert(*id_v1, ent_svc.rid);
            }

            let entconf = res.get_id::<EntertainmentConfiguration>(area)?;

            let lights = match v1.lights {
                HueStreamLightsV1::Rgb(rgb16_v1s) => {
                    let mut v2 = vec![];

                    for rgb in rgb16_v1s {
                        let ent = mapping[&rgb.light_id];
                        for chan in &entconf.channels {
                            for member in &chan.members {
                                if member.service.rid != ent {
                                    continue;
                                }

                                v2.push(Rgb16V2 {
                                    channel: u8::try_from(chan.channel_id)?,
                                    rgb: rgb.rgb,
                                });

                                // there's almost always just a single
                                // member for each channel, but we don't
                                // want to add the light multiple times
                                break;
                            }
                        }
                    }

                    HueStreamLightsV2::Rgb(v2)
                }
                HueStreamLightsV1::Xy(xy16_v1s) => {
                    let mut v2 = vec![];

                    for xy in xy16_v1s {
                        let ent = mapping[&xy.light_id];
                        for chan in &entconf.channels {
                            for member in &chan.members {
                                if member.service.rid != ent {
                                    continue;
                                }

                                v2.push(Xy16V2 {
                                    channel: u8::try_from(chan.channel_id)?,
                                    xy: xy.xy,
                                });

                                // there's almost always just a single
                                // member for each channel, but we don't
                                // want to add the light multiple times
                                break;
                            }
                        }
                    }

                    HueStreamLightsV2::Xy(v2)
                }
            };

            Ok(HueStreamPacketV2 { area, lights })
        } else {
            Err(ApiError::EntStreamInitError)
        }
    }

    pub fn translate_frame(res: &Resources, pkt: HueStreamPacket) -> ApiResult<HueStreamPacketV2> {
        match pkt {
            HueStreamPacket::V1(v1) => Self::translate_v1_frame(res, v1),
            HueStreamPacket::V2(v2) => Ok(v2),
        }
    }

    /// Accept the DTLS handshake, then stream until the sender stops.
    pub async fn run(&self, mut sess: SslStream<UdpStream>) -> ApiResult<()> {
        let mut buf = [0u8; 1024];

        timeout(Duration::from_secs(5), Pin::new(&mut sess).accept())
            .await
            .map_err(|_| ApiError::EntStreamTimeout)??;

        // read the first frame, and use it to look up area, color mode, etc.
        // this means we discard the first frame, but since we expect at least
        // 10 frames *per second*, this is acceptable.
        let sz = Self::read_frame(&mut sess, &mut buf).await?;
        log::trace!("First entertainment frame: {}", hex::encode(&buf[..sz]));
        let raw = HueStreamPacket::parse(&buf[..sz])?;

        let lock = self.res.lock().await;

        let header = Self::translate_frame(&lock, raw)?;

        // look up entertainment area, to make sure it exists
        let _ent: &EntertainmentConfiguration = lock.get_id(header.area)?;

        // request entertainment mode start
        lock.backend_request(BackendRequest::EntertainmentStart(header.area))?;

        drop(lock);

        let area = header.area;
        log::info!("Entertainment stream started for area {area}");
        self.streams.open(area);

        let result = self.pump(&mut sess, &mut buf, sz, &header).await;

        // the area is the only thing that names what to stop, and it is known
        // from here on, so the stop is sent whatever ended the loop
        if let Err(err) = self
            .res
            .lock()
            .await
            .backend_request(BackendRequest::EntertainmentStop(area))
        {
            log::warn!("Could not announce the end of stream {area}: {err}");
        }

        match &result {
            Ok(()) => self.streams.close(&area),
            // a stream that ended badly stays listed, with its reason, so the
            // diagnostics screen can say why the TV's picture went dark
            Err(err) => self.streams.set_error(&area, &err.to_string()),
        }

        result
    }

    /// Forward every frame of one session until the sender stops.
    async fn pump(
        &self,
        sess: &mut SslStream<UdpStream>,
        buf: &mut [u8],
        mut sz: usize,
        header: &HueStreamPacketV2,
    ) -> ApiResult<()> {
        let mut fps = 0;
        let mut period = Utc::now().timestamp();

        loop {
            // a stop from the admin screen ends the session here, on its
            // next frame, rather than leaving the sender streaming into a
            // backend that has already forgotten it
            if self.streams.stop_requested(&header.area) {
                log::info!("Stopping entertainment stream for area {}", header.area);
                break;
            }

            let view = &buf[..sz];
            log::trace!("Packet buffer: {}", view.escape_ascii());

            let raw = HueStreamPacket::parse(view)?;
            let pkt = Self::translate_frame(&*self.res.lock().await, raw)?;

            if pkt.color_mode() != header.color_mode() {
                log::error!("Entertainment Mode color_mode changed mid-stream.");
                return Err(ApiError::EntStreamDesync);
            }

            if pkt.area != header.area {
                log::error!("Entertainment Mode area changed mid-stream.");
                return Err(ApiError::EntStreamDesync);
            }

            let ts = Utc::now().timestamp();
            if period != ts {
                log::debug!("Incoming entertainment fps: {fps}");
                period = ts;
                fps = 0;
            }

            fps += 1;
            self.streams.frame_in(&pkt.area);
            let req = BackendRequest::EntertainmentFrame(pkt.area, pkt.lights);
            self.res.lock().await.backend_request(req)?;

            sz = Self::read_frame(sess, buf).await?;
            if sz == 0 {
                break;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Service for EntertainmentService {
    type Error = ApiError;

    async fn configure(&mut self) -> Result<(), Self::Error> {
        let mut bldr = SslContext::builder(SslMethod::dtls_server())?;

        bldr.set_psk_server_callback(|_sslref, cid, psk| {
            let client_id = String::from_utf8_lossy(cid.unwrap_or_default());
            log::debug!("Setting PSK for {client_id}",);
            STANDARD_CLIENT_KEY.write_to_slice(psk).unwrap();

            log::trace!("psk: {}", hex::encode(&psk[..16]));
            Ok(16)
        });

        self.ctx = Some(Arc::new(bldr.build()));
        Ok(())
    }

    async fn start(&mut self) -> Result<(), Self::Error> {
        let socket = UdpSocket::bind(self.addr).await?;
        // We need a very small receive buffer, since we deliberately want to
        // drop packets if we can't keep up with the sync mode packets
        socket::setsockopt(&socket.as_fd(), RcvBuf, &512)?;

        let listener = UdpListenBuilder::new(socket)
            .with_buffer_size(512)
            .listen()
            .await?;
        self.udp = Some(Arc::new(listener));
        Ok(())
    }

    async fn run(&mut self) -> Result<(), Self::Error> {
        let Some(udp) = self.udp.clone() else {
            return Err(ApiError::service_error("Udp not initialized"));
        };

        let Some(ctx) = self.ctx.clone() else {
            return Err(ApiError::service_error("Ctx not initialized"));
        };

        let session = Session {
            res: self.res.clone(),
            streams: self.streams.clone(),
        };

        // one task per session, reaped here: a sender that panics ends its
        // own task and the listener keeps accepting
        let mut sessions = JoinSet::new();

        loop {
            tokio::select! {
                accepted = udp.accept() => {
                    let (socket, addr) = accepted?;

                    let limit = self.max_streams();
                    if sessions.len() >= limit {
                        log::warn!(
                            "Refusing an entertainment stream from {addr}: {limit} already running"
                        );
                        // dropping the socket leaves the sender without a
                        // handshake, which is how a busy bridge answers
                        drop(socket);
                        continue;
                    }

                    let ssl = Ssl::new(&ctx)?;
                    let stream = SslStream::new(ssl, socket)?;
                    let session = session.clone();

                    sessions.spawn(async move {
                        let result = session.run(stream).await;
                        (addr, result)
                    });
                }

                Some(finished) = sessions.join_next(), if !sessions.is_empty() => {
                    match finished {
                        Ok((addr, Ok(()))) => {
                            log::info!("Entertainment stream from {addr} finished");
                        }
                        Ok((addr, Err(err))) => {
                            log::error!("Entertainment stream from {addr} error: {err}");
                        }
                        Err(err) if err.is_panic() => {
                            log::error!("Entertainment stream panicked; still listening");
                        }
                        Err(err) => {
                            log::error!("Entertainment stream task failed: {err}");
                        }
                    }
                }
            }
        }
    }

    async fn stop(&mut self) -> Result<(), Self::Error> {
        self.udp.take();
        Ok(())
    }
}
