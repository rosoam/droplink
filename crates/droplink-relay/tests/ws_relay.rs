use droplink_core::relay::{encode_binary, ClientMsg, ServerMsg};
use droplink_relay::room::Rooms;
use droplink_relay::server::{serve, AppState};
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_json(ws: &mut Ws) -> ServerMsg {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout")
            .expect("stream ended")
            .expect("ws error");
        match message {
            Message::Text(text) => {
                return serde_json::from_str(text.as_str()).expect("invalid server message")
            }
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected message: {other:?}"),
        }
    }
}

async fn send_json(ws: &mut Ws, message: &ClientMsg) {
    let json = serde_json::to_string(message).unwrap();
    ws.send(Message::Text(json.into())).await.unwrap();
}

async fn register(ws: &mut Ws, id: &str, alias: &str) -> String {
    send_json(
        ws,
        &ClientMsg::Register {
            device_id: id.to_string(),
            alias: alias.to_string(),
            room: Some("test-room".to_string()),
        },
    )
    .await;
    loop {
        match next_json(ws).await {
            ServerMsg::Registered { peer_id } => return peer_id,
            ServerMsg::RoomUpdate { .. } => continue,
            other => panic!("expected registered, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_pairs_and_forwards() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state: AppState = Arc::new(Mutex::new(Rooms::new()));
    let server = tokio::spawn(serve(listener, state, std::future::pending()));

    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut a, _) = connect_async(url.clone()).await.unwrap();
    let (mut b, _) = connect_async(url).await.unwrap();

    let id_a = register(&mut a, "11111111-1111-4111-8111-111111111111", "A").await;
    let id_b = register(&mut b, "22222222-2222-4222-8222-222222222222", "B").await;

    // A gets a room update containing B.
    let update = next_json(&mut a).await;
    let peer_b = match update {
        ServerMsg::RoomUpdate { peers } => peers
            .into_iter()
            .find(|peer| peer.peer_id == id_b)
            .expect("B missing from room update"),
        other => panic!("expected room update, got {other:?}"),
    };
    assert_eq!(peer_b.alias, "B");

    send_json(
        &mut a,
        &ClientMsg::Connect {
            peer_id: id_b.clone(),
        },
    )
    .await;

    let circuit = match next_json(&mut a).await {
        ServerMsg::CircuitOpened {
            circuit_id,
            initiator,
            ..
        } => {
            assert!(initiator);
            circuit_id
        }
        other => panic!("expected circuit opened, got {other:?}"),
    };
    match next_json(&mut b).await {
        ServerMsg::CircuitOpened {
            circuit_id,
            initiator,
            ..
        } => {
            assert_eq!(circuit_id, circuit);
            assert!(!initiator);
        }
        other => panic!("expected circuit opened, got {other:?}"),
    }

    let payload = b"end-to-end-ciphertext-marker";
    let frame = encode_binary(&circuit, payload).unwrap();
    a.send(Message::Binary(frame.clone().into())).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match b.next().await {
                Some(Ok(Message::Binary(bytes))) => return bytes,
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                other => panic!("expected binary, got {other:?}"),
            }
        }
    })
    .await
    .expect("timeout");
    assert_eq!(received.as_ref(), frame.as_slice());

    assert_eq!(id_a.len(), 36);
    drop(b);

    let closed = next_json(&mut a).await;
    match closed {
        ServerMsg::CircuitClosed { circuit_id, .. } => assert_eq!(circuit_id, circuit),
        other => panic!("expected circuit closed, got {other:?}"),
    }

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_rejects_duplicate_device_id() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state: AppState = Arc::new(Mutex::new(Rooms::new()));
    let server = tokio::spawn(serve(listener, state, std::future::pending()));

    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut a, _) = connect_async(url.clone()).await.unwrap();
    let (mut b, _) = connect_async(url).await.unwrap();

    let _ = register(&mut a, "11111111-1111-4111-8111-111111111111", "A").await;
    send_json(
        &mut b,
        &ClientMsg::Register {
            device_id: "11111111-1111-4111-8111-111111111111".into(),
            alias: "B".into(),
            room: Some("test-room".into()),
        },
    )
    .await;
    match next_json(&mut b).await {
        ServerMsg::Error { code, .. } => assert_eq!(code, "duplicate_id"),
        other => panic!("expected error, got {other:?}"),
    }

    server.abort();
}
