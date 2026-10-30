//! A stand-in for a zigbee2mqtt websocket, for the gate scripts.
//!
//! Every connection first receives the messages in `--devices` (a JSON array
//! of `{"topic", "payload"}` objects), then everything the client sends is
//! appended to `--log`, one JSON message per line, so a script can assert
//! what Bifrost asked zigbee2mqtt to do.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Mutex};

use camino::Utf8PathBuf;
use clap::Parser;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

/// Boxed rather than `ApiResult`: `ApiError` is large enough to trip
/// `clippy::result_large_err`, and a test fixture needs no error taxonomy.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Parser, Debug)]
struct Args {
    /// Port to listen on, on 127.0.0.1
    #[arg(long)]
    port: u16,

    /// JSON array of messages sent to every client on connect
    #[arg(long)]
    devices: Utf8PathBuf,

    /// File every message received from a client is appended to
    #[arg(long)]
    log: Utf8PathBuf,
}

async fn serve(
    stream: TcpStream,
    greeting: Arc<Vec<String>>,
    log: Arc<Mutex<fs::File>>,
) -> Result<()> {
    let mut socket = tokio_tungstenite::accept_async(stream).await?;

    for message in greeting.iter() {
        socket.send(Message::text(message.as_str())).await?;
    }

    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                let mut file = log.lock().unwrap();
                writeln!(file, "{}", text.as_str())?;
                file.flush()?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let messages: Vec<Value> = serde_json::from_str(&fs::read_to_string(&args.devices)?)?;
    let greeting = Arc::new(
        messages
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()?,
    );
    let log = Arc::new(Mutex::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&args.log)?,
    ));

    let listener = TcpListener::bind(("127.0.0.1", args.port)).await?;
    eprintln!("mock-z2m: listening on 127.0.0.1:{}", args.port);

    loop {
        let (stream, _) = listener.accept().await?;
        let (greeting, log) = (greeting.clone(), log.clone());
        tokio::spawn(async move {
            if let Err(err) = serve(stream, greeting, log).await {
                eprintln!("mock-z2m: connection ended: {err}");
            }
        });
    }
}
