//! The mock zigbee2mqtt harness: replay, requests and answers, recording.

mod support;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use support::mock_z2m::{MockZ2m, fixture_path};

/// Connect a raw websocket client to the mock, as Bifrost's backend does.
async fn connect(
    mock: &MockZ2m,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (socket, _) = tokio_tungstenite::connect_async(mock.url()).await.unwrap();
    socket
}

async fn next_message(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let message = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .expect("the mock sent nothing")
        .expect("the mock closed the connection")
        .unwrap();
    let Message::Text(text) = message else {
        panic!("the mock sent a non-text message: {message:?}");
    };
    serde_json::from_str(text.as_str()).unwrap()
}

async fn send(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    topic: &str,
    payload: Value,
) {
    let text = json!({"topic": topic, "payload": payload}).to_string();
    socket.send(Message::text(text)).await.unwrap();
}

/// Every fixture message must parse as a z2m message, so a fixture cannot
/// drift away from the protocol the backend speaks.
#[test]
fn wave_3_fixture_parses_as_z2m_messages() {
    let path = fixture_path("w03/z2m/bridge.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let messages: Vec<Value> = serde_json::from_str(&text).unwrap();

    for message in &messages {
        // from_str, not from_value: IeeeAddress borrows from the input
        let text = serde_json::to_string(message).unwrap();
        let topic = message["topic"].as_str().unwrap();

        if topic.starts_with("bridge/") {
            serde_json::from_str::<z2m::api::Message>(&text)
                .unwrap_or_else(|err| panic!("{topic} does not parse: {err}"));
        } else {
            // a device topic is a raw message; its payload is a state report
            let raw: z2m::api::RawMessage = serde_json::from_str(&text).unwrap();
            assert!(raw.payload.is_object(), "{topic} has no state object");
        }
    }

    let topics: Vec<&str> = messages
        .iter()
        .map(|message| message["topic"].as_str().unwrap())
        .collect();
    assert_eq!(
        topics,
        [
            "bridge/state",
            "bridge/info",
            "bridge/devices",
            "bridge/groups",
            "Kitchen ceiling",
            "Kitchen ceiling/availability",
            "Hall lamp",
            "Hall lamp/availability",
        ]
    );
}

#[tokio::test]
async fn replays_the_fixture_in_order_on_connect() {
    let mock = MockZ2m::start_w03().await.unwrap();
    let mut socket = connect(&mock).await;

    let mut topics = vec![];
    for _ in 0..8 {
        topics.push(
            next_message(&mut socket).await["topic"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    assert_eq!(topics[0], "bridge/state");
    assert_eq!(topics[2], "bridge/devices");
    assert_eq!(topics[7], "Hall lamp/availability");
}

#[tokio::test]
async fn answers_a_bridge_request_on_its_response_topic() {
    let mock = MockZ2m::start_w03().await.unwrap();
    let mut socket = connect(&mock).await;
    for _ in 0..8 {
        next_message(&mut socket).await;
    }

    send(
        &mut socket,
        "bridge/request/permit_join",
        json!({"time": 60}),
    )
    .await;
    let answer = next_message(&mut socket).await;
    assert_eq!(answer["topic"], "bridge/response/permit_join");
    assert_eq!(answer["payload"]["status"], "ok");
    assert_eq!(answer["payload"]["data"]["time"], 60);

    let change = json!({"device": "Hall lamp", "group": "Kitchen"});
    send(
        &mut socket,
        "bridge/request/group/members/add",
        change.clone(),
    )
    .await;
    let answer = next_message(&mut socket).await;
    assert_eq!(answer["topic"], "bridge/response/group/members/add");
    assert_eq!(answer["payload"]["data"], change);

    // scene requests follow the same rule, whatever the backend sends
    send(&mut socket, "bridge/request/scene/store", json!({"ID": 3})).await;
    let answer = next_message(&mut socket).await;
    assert_eq!(answer["topic"], "bridge/response/scene/store");
}

#[tokio::test]
async fn records_device_updates_without_answering_them() {
    let mock = MockZ2m::start_w03().await.unwrap();
    let mut socket = connect(&mock).await;
    for _ in 0..8 {
        next_message(&mut socket).await;
    }

    send(&mut socket, "Kitchen ceiling/set", json!({"state": "ON"})).await;

    let payload = mock.wait_for("Kitchen ceiling/set").await;
    assert_eq!(payload, json!({"state": "ON"}));
    assert_eq!(
        mock.received_on("Kitchen ceiling/set"),
        [json!({"state": "ON"})]
    );
    // no answer is due on a device topic, so the socket stays quiet
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), socket.next())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn publish_reaches_a_connected_client() {
    let mock = MockZ2m::start_w03().await.unwrap();
    let mut socket = connect(&mock).await;
    for _ in 0..8 {
        next_message(&mut socket).await;
    }

    mock.publish("Hall lamp/availability", &json!({"state": "offline"}));

    let message = next_message(&mut socket).await;
    assert_eq!(message["topic"], "Hall lamp/availability");
    assert_eq!(message["payload"]["state"], "offline");
}

#[tokio::test]
async fn wait_for_ignores_messages_that_arrived_earlier() {
    let mock = MockZ2m::start_w03().await.unwrap();
    let mut socket = connect(&mock).await;
    for _ in 0..8 {
        next_message(&mut socket).await;
    }
    send(&mut socket, "Hall lamp/set", json!({"state": "OFF"})).await;
    mock.wait_for("Hall lamp/set").await;

    assert!(
        mock.wait_for_timeout("Hall lamp/set", std::time::Duration::from_millis(100))
            .await
            .is_none(),
        "wait_for returned a message that had already arrived"
    );
}
