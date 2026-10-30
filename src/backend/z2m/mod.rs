mod backend_event;
mod bridge_event;
mod bridge_import;
pub mod discover;
pub mod entertainment;
pub mod learn;
pub mod websocket;
pub mod zclcommand;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use native_tls::TlsConnector;
use svc::error::SvcError;
use svc::template::ServiceTemplate;
use svc::traits::{BoxDynService, Service};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
};
use uuid::Uuid;

use bifrost_api::backend::BackendRequest;
use hue::api::ResourceLink;
use z2m::update::DeviceUpdate;

use crate::backend::z2m::entertainment::AreaStream;
use crate::backend::z2m::learn::SceneLearn;
use crate::backend::z2m::websocket::Z2mWebSocket;
use crate::config::Z2mServer;
use crate::config_store::ConfigStore;
use crate::error::{ApiError, ApiResult};
use crate::inventory::{GroupRef, Inventory};
use crate::resource::Resources;
use crate::server::appstate::AppState;

#[derive(Error, Debug)]
pub enum TemplateError {
    #[error("No config found for z2m server {0:?}")]
    NotFound(String),
}

/// A command addressed to exactly one z2m backend.
///
/// The admin API reaches a backend through the channel registered under its
/// server name, so a command never reaches the other servers. Each command
/// carries the way to answer, so a route can report the failure instead of
/// guessing.
#[derive(Debug)]
pub enum Z2mCommand {
    /// Make one device flash, so a person can tell which bulb it is.
    Identify {
        device: String,
        seconds: u32,
        reply: oneshot::Sender<ApiResult<()>>,
    },
    /// Open this server's join window, and no other server's.
    PermitJoin {
        time: u32,
        device: Option<String>,
        reply: oneshot::Sender<ApiResult<()>>,
    },
}

/// The command channel of every running backend, by server name.
#[derive(Debug, Default)]
pub struct BackendCommands {
    next_token: AtomicU64,
    senders: StdMutex<HashMap<String, (u64, mpsc::UnboundedSender<Z2mCommand>)>>,
}

impl BackendCommands {
    /// Claim a channel for the backend named `server`.
    ///
    /// The returned token identifies this instance, so a backend that is
    /// being replaced does not deregister its successor.
    pub fn register(&self, server: &str) -> (u64, mpsc::UnboundedReceiver<Z2mCommand>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);

        let mut senders = self.senders.lock().unwrap_or_else(PoisonError::into_inner);
        senders.insert(server.to_string(), (token, sender));
        drop(senders);

        (token, receiver)
    }

    /// Give up the channel claimed by `token`, if it is still the current one.
    pub fn unregister(&self, server: &str, token: u64) {
        let mut senders = self.senders.lock().unwrap_or_else(PoisonError::into_inner);
        if senders
            .get(server)
            .is_some_and(|(current, _)| *current == token)
        {
            senders.remove(server);
        }
    }

    /// The channel of the running backend named `server`.
    #[must_use]
    pub fn sender(&self, server: &str) -> Option<mpsc::UnboundedSender<Z2mCommand>> {
        let senders = self.senders.lock().unwrap_or_else(PoisonError::into_inner);
        senders.get(server).map(|(_, sender)| sender.clone())
    }

    /// The names of the backends that are reachable right now.
    #[must_use]
    pub fn running(&self) -> Vec<String> {
        let senders = self.senders.lock().unwrap_or_else(PoisonError::into_inner);
        senders.keys().cloned().collect()
    }
}

pub struct Z2mServiceTemplate {
    state: AppState,
}

impl Z2mServiceTemplate {
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl ServiceTemplate for Z2mServiceTemplate {
    fn generate(&self, name: String) -> Result<BoxDynService, SvcError> {
        let config = self.state.config();
        let Some(server) = config.z2m.servers.get(&name) else {
            return Err(SvcError::generation(TemplateError::NotFound(name)));
        };
        let svc = Z2mBackend::new(
            name,
            server.clone(),
            self.state.store(),
            self.state.res.clone(),
        )
        .map_err(SvcError::generation)?
        .with_inventory(self.state.inventory())
        .with_commands(self.state.backends());

        Ok(svc.boxed())
    }
}

pub struct Z2mBackend {
    name: String,
    server: Z2mServer,
    store: Arc<ConfigStore>,
    state: Arc<Mutex<Resources>>,
    map: HashMap<String, ResourceLink>,
    rmap: HashMap<ResourceLink, String>,
    learner: SceneLearn,
    ignore: HashSet<String>,
    network: HashMap<String, z2m::api::Device>,
    /// The z2m group each device belongs to, by IEEE address.
    group_of: HashMap<String, GroupRef>,
    /// The id of each z2m group, by friendly name.
    group_ids: HashMap<String, u32>,
    /// One stream per entertainment area. Two areas run side by side, each
    /// with its own throttle and its own Zigbee sequence counter: a shared
    /// 20 fps budget would make "independent fps" a lie.
    entstreams: BTreeMap<Uuid, AreaStream>,
    /// The Zigbee sequence counter each area stopped at, so a stream that
    /// starts again continues where it left off.
    counters: BTreeMap<Uuid, u32>,
    fps: u32,
    socket: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    /// Where what this backend learns is published. Absent in a unit test
    /// that drives the backend without one.
    inventory: Option<Arc<Inventory>>,
    /// The channel the admin API reaches this backend on.
    commands: Option<Arc<BackendCommands>>,
    /// This instance's claim on that channel, taken when it starts.
    command_token: Option<u64>,
    command_rx: Option<mpsc::UnboundedReceiver<Z2mCommand>>,

    // for sending delayed messages over the websocket
    message_rx: mpsc::UnboundedReceiver<(String, DeviceUpdate)>,
    message_tx: mpsc::UnboundedSender<(String, DeviceUpdate)>,
}

impl Z2mBackend {
    const DEFAULT_FPS: u32 = 20;
    const LIGHT_BREATHE_DURATION: Duration = Duration::from_secs(2);

    pub fn new(
        name: String,
        server: Z2mServer,
        store: Arc<ConfigStore>,
        state: Arc<Mutex<Resources>>,
    ) -> ApiResult<Self> {
        let fps = server.streaming_fps.map_or(Self::DEFAULT_FPS, u32::from);
        let map = HashMap::new();
        let rmap = HashMap::new();
        let ignore = HashSet::new();
        let learner = SceneLearn::new(name.clone());
        let network = HashMap::new();
        let group_of = HashMap::new();
        let group_ids = HashMap::new();
        let entstreams = BTreeMap::new();
        let counters = BTreeMap::new();
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        Ok(Self {
            name,
            server,
            store,
            state,
            map,
            rmap,
            learner,
            ignore,
            network,
            group_of,
            group_ids,
            entstreams,
            counters,
            fps,
            message_rx,
            message_tx,
            socket: None,
            inventory: None,
            commands: None,
            command_token: None,
            command_rx: None,
        })
    }

    /// Publish what this backend learns into `inventory`.
    ///
    /// A separate step rather than another argument to [`Self::new`]: the
    /// constructor stays about the server's own configuration.
    #[must_use]
    pub fn with_inventory(mut self, inventory: Arc<Inventory>) -> Self {
        self.inventory = Some(inventory);
        self
    }

    /// Let the admin API reach this backend by its server name.
    #[must_use]
    pub fn with_commands(mut self, commands: Arc<BackendCommands>) -> Self {
        self.commands = Some(commands);
        self
    }

    /// The registry this backend publishes into, when it has one.
    pub(crate) const fn inventory(&self) -> Option<&Arc<Inventory>> {
        self.inventory.as_ref()
    }

    /// Give up this instance's claim on its command channel.
    fn release_commands(&mut self) {
        let (Some(commands), Some(token)) = (&self.commands, self.command_token.take()) else {
            return;
        };
        commands.unregister(&self.name, token);
        self.command_rx = None;
    }

    pub async fn event_loop(
        &mut self,
        chan: &mut Receiver<Arc<BackendRequest>>,
        frames: &mut Receiver<Arc<BackendRequest>>,
        mut socket: Z2mWebSocket,
        commands: Option<&mut mpsc::UnboundedReceiver<Z2mCommand>>,
    ) -> ApiResult<()> {
        let mut commands = commands;
        let mut frames_open = true;
        loop {
            select! {
                // all backend event handling implemented in backend::z2m::backend_event
                pkt = chan.recv() => {
                    match pkt {
                        Ok(api_req) => self.handle_backend_event(&mut socket, api_req).await?,
                        // the control channel never carries frames, so lag
                        // here means this backend is behind on state changes;
                        // dropping them beats ending the loop
                        Err(broadcast::error::RecvError::Lagged(missed)) => {
                            log::warn!("[{}] Backend events lagged, dropped {missed}", self.name);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return Err(ApiError::UnexpectedZ2mEof);
                        }
                    }
                },

                // entertainment frames, on their own channel: a backend that
                // falls behind drops frames and keeps running
                frame = recv_frame(&mut frames_open, frames) => {
                    match frame {
                        Some(Ok(frame)) => self.handle_backend_event(&mut socket, frame).await?,
                        Some(Err(broadcast::error::RecvError::Lagged(missed))) => {
                            log::debug!("[{}] Dropped {missed} entertainment frames", self.name);
                        }
                        Some(Err(broadcast::error::RecvError::Closed)) | None => {
                            frames_open = false;
                        }
                    }
                },

                // all bridge event handling implemented in backend::z2m::bridge_event
                pkt = socket.next() => {
                    self.handle_bridge_event(pkt.ok_or(ApiError::UnexpectedZ2mEof)??).await?;
                },

                Some((topic, upd)) = self.message_rx.recv() => {
                    socket.send_update(&topic, &upd).await?;
                }

                // a command from the admin API, addressed to this server only
                Some(command) = recv_command(&mut commands) => {
                    self.handle_command(&mut socket, command).await;
                }
            };
        }
    }

    /// Carry out a command addressed to this backend and answer it.
    ///
    /// The answer is the send result, so a route reports a broken socket
    /// rather than claiming the device flashed.
    async fn handle_command(&self, z2mws: &mut Z2mWebSocket, command: Z2mCommand) {
        match command {
            Z2mCommand::Identify {
                device,
                seconds,
                reply,
            } => {
                let result = z2mws.send_identify(&device, seconds).await;
                let _ = reply.send(result);
            }
            Z2mCommand::PermitJoin {
                time,
                device,
                reply,
            } => {
                let result = z2mws.send_permit_join(time, device).await;
                let _ = reply.send(result);
            }
        }
    }
}

/// Await the next command, or nothing at all when this backend has no
/// channel.
async fn recv_command(
    commands: &mut Option<&mut mpsc::UnboundedReceiver<Z2mCommand>>,
) -> Option<Z2mCommand> {
    match commands {
        Some(commands) => commands.recv().await,
        None => std::future::pending().await,
    }
}

/// Await the next frame, or nothing once the frame channel is closed.
///
/// A closed channel returns immediately on every poll, which would spin the
/// event loop; the flag turns that branch into one that never completes.
async fn recv_frame(
    open: &mut bool,
    frames: &mut Receiver<Arc<BackendRequest>>,
) -> Option<Result<Arc<BackendRequest>, broadcast::error::RecvError>> {
    if !*open {
        return std::future::pending().await;
    }

    Some(frames.recv().await)
}

#[async_trait]
impl Service for Z2mBackend {
    type Error = ApiError;

    async fn start(&mut self) -> ApiResult<()> {
        // let's not include auth tokens in log output
        let sanitized_url = self.server.get_sanitized_url();
        let url = self.server.get_url();

        // a token from the token field is appended by design; only a path
        // rewrite is worth a hint then
        let rewritten = if self.server.token.is_some() {
            url.path() != self.server.url.path()
        } else {
            url != self.server.url
        };

        if rewritten {
            log::info!(
                "[{}] Rewrote url for compatibility with z2m 2.x.",
                self.name
            );
            log::info!(
                "[{}] Consider updating websocket url to {}",
                self.name,
                sanitized_url
            );
        }

        // if tls verification is disabled, build a TlsConnector that explicitly
        // does not check certificate validity. This is obviously neither safe
        // nor recommended.
        let connector = if self.server.disable_tls_verify.unwrap_or_default() {
            log::warn!(
                "[{}] TLS verification disabled; will accept any certificate!",
                self.name
            );
            Some(Connector::NativeTls(
                TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .build()?,
            ))
        } else {
            None
        };

        log::info!("[{}] Connecting to {}", self.name, &sanitized_url);
        match connect_async_tls_with_config(url.as_str(), None, false, connector.clone()).await {
            Ok((socket, _)) => {
                self.socket = Some(socket);
                Ok(())
            }
            Err(err) => {
                log::error!("[{}] Connect failed: {err:?}", self.name);
                Err(err.into())
            }
        }
    }

    async fn run(&mut self) -> ApiResult<()> {
        if let Some(socket) = self.socket.take() {
            let inventory = self.inventory.clone();
            let commands = self.commands.clone();

            if let Some(inventory) = &inventory {
                inventory.server_up(&self.name);
            }
            if let Some(commands) = &commands {
                let (token, receiver) = commands.register(&self.name);
                self.command_token = Some(token);
                self.command_rx = Some(receiver);
            }

            let z2m_socket = Z2mWebSocket::new(self.name.clone(), socket);
            let (mut chan, mut frames) = {
                let lock = self.state.lock().await;
                (lock.backend_event_stream(), lock.frame_stream())
            };
            let mut command_rx = self.command_rx.take();
            let res = self
                .event_loop(&mut chan, &mut frames, z2m_socket, command_rx.as_mut())
                .await;
            self.command_rx = command_rx;

            if let Err(err) = res {
                log::error!("[{}] Event loop broke: {err}", self.name);
            }

            // a backend that stopped is not gone: its devices stay listed,
            // but nothing is known to be reachable any more
            if let Some(inventory) = &inventory {
                inventory.server_down(&self.name);
            }
            self.release_commands();
        }
        Ok(())
    }

    async fn stop(&mut self) -> ApiResult<()> {
        self.socket.take();
        if let Some(inventory) = &self.inventory {
            inventory.server_down(&self.name);
        }
        self.release_commands();
        Ok(())
    }
}
