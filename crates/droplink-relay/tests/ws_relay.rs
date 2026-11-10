use droplink_core::protocol::PROTOCOL_VERSION;
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

async fn create_session(ws: &mut Ws, id: &str, alias: &str) -> (String, String) {
    send_json(
        ws,
        &ClientMsg::CreateSession {
            proto: PROTOCOL_VERSION,
            device_id: id.to_string(),
            alias: alias.to_string(),
        },
    )
    .await;
    loop {
        match next_json(ws).await {
            ServerMsg::SessionCreated { code, token, .. } => return (code, token),
            ServerMsg::RoomUpdate { .. } => continue,
            other => panic!("expected session created, got {other:?}"),
        }
    }
}

async fn start() -> (u16, tokio::task::JoinHandle<std::io::Result<()>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state: AppState = Arc::new(Mutex::new(Rooms::new()));
    let server = tokio::spawn(serve(listener, state, std::future::pending()));
    (port, server)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_pairs_and_forwards() {
    let (port, server) = start().await;

    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut a, _) = connect_async(url.clone()).await.unwrap();
    let (mut b, _) = connect_async(url).await.unwrap();

    let (code, _token) = create_session(&mut a, "11111111-1111-4111-8111-111111111111", "A").await;
    send_json(
        &mut b,
        &ClientMsg::JoinSession {
            proto: PROTOCOL_VERSION,
            device_id: "22222222-2222-4222-8222-222222222222".into(),
            alias: "B".into(),
            code,
        },
    )
    .await;

    loop {
        match next_json(&mut b).await {
            ServerMsg::SessionJoined { .. } => break,
            ServerMsg::RoomUpdate { .. } => continue,
            other => panic!("expected session joined, got {other:?}"),
        }
    }

    let update = match next_json(&mut a).await {
        ServerMsg::RoomUpdate { peers } => peers,
        other => panic!("expected room update, got {other:?}"),
    };
    let peer_b = update
        .first()
        .expect("B missing from room update")
        .peer_id
        .clone();

    send_json(
        &mut a,
        &ClientMsg::Connect {
            peer_id: peer_b.clone(),
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

    drop(b);

    let closed = loop {
        match next_json(&mut a).await {
            ServerMsg::CircuitClosed { circuit_id, .. } => break circuit_id,
            ServerMsg::RoomUpdate { .. } => continue,
            other => panic!("expected circuit closed, got {other:?}"),
        }
    };
    assert_eq!(closed, circuit);

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_refuses_a_second_session_for_the_same_device() {
    let (port, server) = start().await;

    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut a, _) = connect_async(url.clone()).await.unwrap();
    let (mut b, _) = connect_async(url).await.unwrap();

    let _ = create_session(&mut a, "11111111-1111-4111-8111-111111111111", "A").await;
    send_json(
        &mut b,
        &ClientMsg::CreateSession {
            proto: PROTOCOL_VERSION,
            device_id: "11111111-1111-4111-8111-111111111111".into(),
            alias: "B".into(),
        },
    )
    .await;
    match next_json(&mut b).await {
        ServerMsg::Error { code, .. } => assert_eq!(code, "already_active"),
        other => panic!("expected error, got {other:?}"),
    }

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_rate_limits_session_attempts_per_network() {
    let (port, server) = start().await;
    let url = format!("ws://127.0.0.1:{port}/ws");

    let mut last_code = String::new();
    for index in 0..6 {
        let (mut ws, _) = connect_async(url.clone()).await.unwrap();
        send_json(
            &mut ws,
            &ClientMsg::CreateSession {
                proto: PROTOCOL_VERSION,
                device_id: format!("aaaaaaaa-1111-4111-8111-{index:012}"),
                alias: format!("P{index}"),
            },
        )
        .await;
        loop {
            match next_json(&mut ws).await {
                ServerMsg::RoomUpdate { .. } => continue,
                ServerMsg::SessionCreated { .. } => {
                    assert!(index < 5, "attempt {index} should have been limited");
                    last_code = "ok".into();
                    break;
                }
                ServerMsg::Error { code, .. } => {
                    assert_eq!(code, "rate_limited");
                    assert_eq!(index, 5);
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }
    assert_eq!(last_code, "ok");

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_code_is_refused_without_enumeration() {
    let (port, server) = start().await;
    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut ws, _) = connect_async(url).await.unwrap();
    send_json(
        &mut ws,
        &ClientMsg::JoinSession {
            proto: PROTOCOL_VERSION,
            device_id: "bbbbbbbb-1111-4111-8111-111111111111".into(),
            alias: "B".into(),
            code: "ZZZZ2345".into(),
        },
    )
    .await;
    match next_json(&mut ws).await {
        ServerMsg::Error { code, .. } => assert_eq!(code, "no_session"),
        other => panic!("expected error, got {other:?}"),
    }
    server.abort();
}
