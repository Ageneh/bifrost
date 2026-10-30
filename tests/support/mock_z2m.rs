//! A stand-in zigbee2mqtt websocket, for integration tests.
//!
//! [`MockZ2m::start`] reads a fixture: a JSON array of `{"topic", "payload"}`
//! messages, the same shape `examples/mock-z2m.rs` replays for the gate
//! scripts. Every client that connects receives all of them, in file order,
//! before anything else.
//!
//! What a client sends is handled by topic:
//!
//! - `bridge/request/<rest>` is answered on `bridge/response/<rest>` with
//!   `{"status": "ok", "data": <the request payload>}`, which is what z2m
//!   answers for `permit_join`, `group/members/add|remove` and any future
//!   `scene/*` request.
//! - everything else is recorded, which covers `<name>/set` (device updates
//!   and the group-level `scene_store`/`scene_recall`/`scene_remove`), and
//!   every `<name>/get`.
//!
//! A test drives the mock through [`MockZ2m::publish`], reads what Bifrost
//! sent with [`MockZ2m::received`], and waits for one message with
//! [`MockZ2m::wait_for`].

// every test binary compiles its own copy of this module, and each uses a
// different part of the handle
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use camino::Utf8Path;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, broadcast};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use z2m::api::RawMessage;

/// How long [`MockZ2m::wait_for`] waits by default.
pub const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// State every connection shares.
struct Shared {
    greeting: Vec<String>,
    received: Mutex<Vec<RawMessage>>,
    connections: AtomicUsize,
    outbound: broadcast::Sender<String>,
    changed: Notify,
}

impl Shared {
    fn record(&self, message: RawMessage) {
        self.received.lock().unwrap().push(message);
        self.changed.notify_waiters();
    }
}

/// A running mock zigbee2mqtt server.
pub struct MockZ2m {
    url: String,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl MockZ2m {
    /// Start a mock on a free loopback port, replaying `fixture` on connect.
    pub async fn start(fixture: &Utf8Path) -> Result<Self> {
        let messages = read_fixture(fixture)?;

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let (outbound, _) = broadcast::channel(64);

        let shared = Arc::new(Shared {
            greeting: messages
                .iter()
                .map(serde_json::to_string)
                .collect::<serde_json::Result<Vec<_>>>()?,
            received: Mutex::new(vec![]),
            connections: AtomicUsize::new(0),
            outbound,
            changed: Notify::new(),
        });

        let task = tokio::spawn(accept_loop(listener, shared.clone()));

        Ok(Self {
            url: format!("ws://127.0.0.1:{port}"),
            shared,
            task,
        })
    }

    /// Start the mock on the wave-3 fixture, `tests/fixtures/w03/z2m/bridge.json`.
    pub async fn start_w03() -> Result<Self> {
        Self::start(&fixture_path("w03/z2m/bridge.json")).await
    }

    /// The websocket url Bifrost is configured with.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Send `topic`/`payload` to every connected client, as z2m does for a
    /// device report or a bridge event.
    pub fn publish(&self, topic: &str, payload: &Value) {
        let message = json!({"topic": topic, "payload": payload});
        // a send error only means no client is connected right now
        let _ = self.shared.outbound.send(message.to_string());
    }

    /// How many clients have connected since the mock started; a backend
    /// that is restarted connects again.
    #[must_use]
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    /// Everything clients have sent so far, in arrival order.
    #[must_use]
    pub fn received(&self) -> Vec<RawMessage> {
        self.shared.received.lock().unwrap().clone()
    }

    /// The payloads clients sent on `topic`.
    #[must_use]
    pub fn received_on(&self, topic: &str) -> Vec<Value> {
        self.received()
            .into_iter()
            .filter(|message| message.topic == topic)
            .map(|message| message.payload)
            .collect()
    }

    /// Wait for a message on `topic` that arrives after this call, and
    /// return its payload.
    ///
    /// # Panics
    ///
    /// Panics on timeout, so a test that expected a message fails with the
    /// topics that did arrive instead of hanging.
    pub async fn wait_for(&self, topic: &str) -> Value {
        self.wait_for_timeout(topic, WAIT_TIMEOUT)
            .await
            .unwrap_or_else(|| {
                let seen: Vec<String> = self.received().into_iter().map(|m| m.topic).collect();
                panic!("no message on {topic:?} within {WAIT_TIMEOUT:?}; received {seen:?}");
            })
    }

    /// Wait for a message on `topic`, or `None` on timeout.
    pub async fn wait_for_timeout(
        &self,
        topic: &str,
        timeout: std::time::Duration,
    ) -> Option<Value> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut seen = self.received().len();

        loop {
            let waiting = self.shared.changed.notified();
            let messages = self.received();
            if let Some(message) = messages.get(seen..).and_then(|new| {
                new.iter()
                    .find(|message| message.topic == topic)
                    .map(|message| message.payload.clone())
            }) {
                return Some(message);
            }
            seen = messages.len();

            if tokio::time::timeout_at(deadline, waiting).await.is_err() {
                return None;
            }
        }
    }
}

impl Drop for MockZ2m {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Read a fixture: a JSON array of `{"topic", "payload"}` objects.
fn read_fixture(fixture: &Utf8Path) -> Result<Vec<RawMessage>> {
    let text = std::fs::read_to_string(fixture)
        .map_err(|err| format!("cannot read z2m fixture {fixture}: {err}"))?;
    Ok(serde_json::from_str(&text)
        .map_err(|err| format!("z2m fixture {fixture} is not a message array: {err}"))?)
}

/// Path of a file under `tests/fixtures/`.
#[must_use]
pub fn fixture_path(name: &str) -> camino::Utf8PathBuf {
    camino::Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

async fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(serve(stream, shared.clone()));
    }
}

async fn serve(stream: TcpStream, shared: Arc<Shared>) -> Result<()> {
    let mut socket = tokio_tungstenite::accept_async(stream).await?;
    shared.connections.fetch_add(1, Ordering::SeqCst);
    let mut outbound = shared.outbound.subscribe();

    for message in &shared.greeting {
        send_text(&mut socket, message).await?;
    }

    loop {
        tokio::select! {
            inbound = socket.next() => {
                let Some(inbound) = inbound else { return Ok(()) };
                match inbound? {
                    Message::Text(text) => handle_text(&mut socket, &shared, text.as_str()).await?,
                    Message::Close(_) => return Ok(()),
                    _ => {}
                }
            }
            published = outbound.recv() => {
                match published {
                    Ok(text) => send_text(&mut socket, &text).await?,
                    // a client that fell behind misses a report, never the connection
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

async fn send_text(
    socket: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    text: &str,
) -> Result<()> {
    socket.send(Message::text(text)).await?;
    Ok(())
}

/// Record an inbound message, answering it when it is a request.
async fn handle_text(
    socket: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    shared: &Shared,
    text: &str,
) -> Result<()> {
    let Ok(message) = serde_json::from_str::<RawMessage>(text) else {
        // a client that sends something else is not a z2m client; ignore it
        return Ok(());
    };

    let response = response_for(&message);
    shared.record(message);

    if let Some((topic, payload)) = response {
        send_text(
            socket,
            &json!({"topic": topic, "payload": payload}).to_string(),
        )
        .await?;
    }

    Ok(())
}

/// The answer z2m gives to `message`, when it is a request.
fn response_for(message: &RawMessage) -> Option<(String, Value)> {
    let rest = message.topic.strip_prefix("bridge/request/")?;
    Some((
        format!("bridge/response/{rest}"),
        json!({"status": "ok", "data": message.payload}),
    ))
}
