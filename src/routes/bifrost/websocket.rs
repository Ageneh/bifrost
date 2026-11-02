use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use tokio::select;
use tokio::sync::broadcast;

use bifrost_api::backend::BackendRequest;
use bifrost_api::entertainment::EntertainmentStatus;
use bifrost_api::inventory::InventoryUpdate;
use bifrost_api::service::Service;
use bifrost_api::websocket::Update;
use hue::event::EventBlock;
use svc::manager::{ServiceEvent, SvmClient};

use crate::config::redact::redacted_app_config;
use crate::inventory::InventoryEvent;
use crate::logbuffer;
use crate::routes::bifrost::BifrostApiResult;
use crate::server::appstate::AppState;
use crate::server::hueevents::HueEventRecord;

struct WebSocketTask {
    state: AppState,
    ws: WebSocket,
    mgr: SvmClient,
}

#[allow(clippy::unnecessary_wraps, clippy::unused_self)]
impl WebSocketTask {
    pub fn new(state: AppState, ws: WebSocket) -> Self {
        let mgr = state.manager();

        Self { state, ws, mgr }
    }

    async fn send(&mut self, value: Update) -> BifrostApiResult<()> {
        let text = serde_json::to_string(&value)?.into();

        self.ws.send(Message::Text(text)).await?;

        Ok(())
    }

    fn handle_websocket_message(&self, msg: &Message) -> BifrostApiResult<Option<Update>> {
        log::trace!("Websocket message: {msg:?}");
        Ok(None)
    }

    /// One control event from a backend.
    ///
    /// Entertainment frames never reach here — they travel on their own
    /// channel and nobody would read a diagnostic screen at 120 fps — and a
    /// socket that fell behind skips what it missed instead of closing.
    fn handle_backend_event(
        &self,
        backend_event: Result<Arc<BackendRequest>, broadcast::error::RecvError>,
    ) -> BifrostApiResult<Option<Update>> {
        match backend_event {
            Ok(event) => {
                log::debug!("Backend event: {event:?}");
                Ok(Some(Update::BackendRequest((*event).clone())))
            }
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                log::warn!("Backend subscriber lagged {missed} events; skipping them");
                Ok(None)
            }
            Err(broadcast::error::RecvError::Closed) => Ok(None),
        }
    }

    /// A change to the inventory. A socket that fell behind re-reads the
    /// snapshot, which is the same state a fresh one would get.
    fn handle_inventory_event(
        &self,
        event: Result<InventoryEvent, broadcast::error::RecvError>,
    ) -> BifrostApiResult<Option<Update>> {
        match event {
            Ok(event) => Ok(Some(Update::Inventory(event.into()))),
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                log::warn!("Inventory subscriber lagged {missed} events; resending the snapshot");
                Ok(Some(Update::Inventory(InventoryUpdate::Snapshot(
                    self.state.inventory().snapshot(),
                ))))
            }
            Err(broadcast::error::RecvError::Closed) => Ok(None),
        }
    }

    /// A stream started, ended or failed.
    fn handle_stream_event(
        &self,
        event: Result<Vec<bifrost_api::entertainment::StreamStats>, broadcast::error::RecvError>,
    ) -> BifrostApiResult<Option<Update>> {
        match event {
            Ok(streams) => Ok(Some(Update::EntertainmentStatus(EntertainmentStatus {
                max_streams: self.state.config().bifrost.entertainment.max_streams,
                streams,
            }))),
            // the rates are in the snapshot, and the next change carries one
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                log::warn!("Stream subscriber lagged {missed} events; skipping them");
                Ok(None)
            }
            Err(broadcast::error::RecvError::Closed) => Ok(None),
        }
    }

    fn handle_hue_event(&self, hue_event: HueEventRecord) -> BifrostApiResult<Option<Update>> {
        log::info!("Hue event: {hue_event:?}");
        Ok(Some(Update::HueEvent(hue_event.block)))
    }

    async fn handle_service_event(
        &mut self,
        service_event: Option<ServiceEvent>,
    ) -> BifrostApiResult<Option<Update>> {
        let Some(service_event) = service_event else {
            log::error!("service event channel broke");
            panic!();
        };

        log::trace!("service event: {service_event:?}");

        let name = self.mgr.lookup_name(service_event.id()).await?;

        let service = Service {
            id: service_event.id(),
            name,
            state: service_event.state(),
            last_error: service_event.error().map(ToString::to_string),
        };

        Ok(Some(Update::ServiceUpdate(service)))
    }

    async fn handle_socket(mut self) -> BifrostApiResult<()> {
        let lock = self.state.res.lock().await;
        let mut backend_events = lock.backend_event_stream();
        let mut hue_events = lock.hue_event_stream().subscribe();
        let hue_state = lock.get_resources();
        drop(lock);

        let mut svc_events = self.mgr.subscribe().await?.1;
        let mut log_events = logbuffer::shared().subscribe();

        // subscribe before the snapshot is taken, so a change that lands in
        // between is queued rather than missed
        let inventory = self.state.inventory();
        let mut inventory_events = inventory.subscribe();

        let link_button = self.state.link_button();
        let mut link_button_events = link_button.subscribe();

        let streams = self.state.streams();
        let mut stream_events = streams.subscribe();

        let mut device_events = self.state.device_events().subscribe();

        let app_config = redacted_app_config(&self.state.config())?;
        self.send(Update::AppConfig(app_config)).await?;

        self.send(Update::HueEvent(EventBlock::add(hue_state)))
            .await?;

        self.send(Update::Inventory(InventoryUpdate::Snapshot(
            inventory.snapshot(),
        )))
        .await?;

        self.send(Update::LinkButton(link_button.status())).await?;

        self.send(Update::EntertainmentStatus(EntertainmentStatus {
            max_streams: self.state.config().bifrost.entertainment.max_streams,
            streams: streams.snapshot(),
        }))
        .await?;

        loop {
            let reply = select! {
                Some(msg) = self.ws.recv() => self.handle_websocket_message(&msg?),
                backend_event = backend_events.recv() => self.handle_backend_event(backend_event),
                service_event = svc_events.recv() => self.handle_service_event(service_event).await,
                log_event = log_events.recv() => Ok(log_event.ok().map(Update::Log)),
                hue_event = hue_events.recv() => self.handle_hue_event(hue_event?),
                inventory_event = inventory_events.recv() => self.handle_inventory_event(inventory_event),
                link_button_event = link_button_events.recv() => Ok(link_button_event.ok().map(Update::LinkButton)),
                stream_event = stream_events.recv() => self.handle_stream_event(stream_event),
                device_event = device_events.recv() => Ok(device_event.ok().map(Update::DeviceEvent)),
            };

            if let Some(reply) = reply? {
                self.send(reply).await?;
            }
        }
    }
}

pub async fn websocket(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|ws| async move {
        let wst = WebSocketTask::new(state, ws);

        match wst.handle_socket().await {
            Ok(()) => {
                log::info!("Websocket closed.");
            }
            Err(err) => {
                log::error!("Websocket error: {err:?}");
            }
        }
    })
}
