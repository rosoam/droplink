use droplink_core::peer::{Peer, Reach};
use droplink_core::relay::decode_binary;
use droplink_net::engine::{Command, EngineConfig, Event};
use droplink_net::spawn;
use droplink_relay::room::Rooms;
use droplink_relay::server::{serve, AppState};
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MARKER: &[u8] = b"DROPLINK-LEAK-MARKER-0123456789ABCDEF-0123456789ABCDEF";

struct Node {
    handle: droplink_net::EngineHandle,
    events: mpsc::Receiver<Event>,
    dir: TempDir,
}

impl Node {
    fn start(name: &str, relay_port: u16) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = EngineConfig::new(
            Uuid::new_v4().to_string(),
            name.to_string(),
            dir.path().join("recv"),
        );
        cfg.relay_enabled = true;
        cfg.relay_url = Some(format!("ws://127.0.0.1:{relay_port}/ws"));
        cfg.room_code = Some("test-room".into());
        let (handle, _bound, events) = spawn(cfg).unwrap();
        Self {
            handle,
            events,
            dir,
        }
    }
}

fn start_relay() -> (u16, JoinHandle<()>) {
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = std_listener.local_addr().unwrap().port();
    std_listener.set_nonblocking(true).unwrap();
    let listener = TcpListener::from_std(std_listener).unwrap();
    let state: AppState = Arc::new(Mutex::new(Rooms::new()));
    let task = tokio::spawn(async move {
        let _ = serve(listener, state, std::future::pending()).await;
    });
    (port, task)
}

async fn wait_relay_peer(events: &mut mpsc::Receiver<Event>) -> Peer {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timeout waiting for a relay peer");
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Some(Event::DevicesChanged(peers))) => {
                if let Some(peer) = peers
                    .into_iter()
                    .find(|peer| matches!(peer.reach, Reach::Relay { .. }))
                {
                    return peer;
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("event channel closed"),
            Err(_) => panic!("timeout waiting for a relay peer"),
        }
    }
}

async fn wait_empty_devices(events: &mut mpsc::Receiver<Event>) -> bool {
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(event) = events.recv().await {
            if let Event::DevicesChanged(peers) = event {
                if peers.is_empty() {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}

async fn wait_completed(events: &mut mpsc::Receiver<Event>) -> Event {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timeout waiting for completion");
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Some(event @ Event::Completed { .. })) => return event,
            Ok(Some(Event::Failed { reason, .. })) => panic!("transfer failed: {reason}"),
            Ok(Some(_)) => {}
            Ok(None) => panic!("channel closed"),
            Err(_) => panic!("timeout"),
        }
    }
}

async fn wait_incoming_and_accept(
    events: &mut mpsc::Receiver<Event>,
    handle: &droplink_net::EngineHandle,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timeout waiting for incoming");
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Some(Event::Incoming(incoming))) => {
                handle.command(Command::Respond {
                    transfer_id: incoming.transfer_id,
                    accept: true,
                });
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("channel closed"),
            Err(_) => panic!("timeout"),
        }
    }
}

fn write_marker_file(dir: &std::path::Path, size: usize) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("leak.bin");
    let mut data = Vec::with_capacity(size);
    while data.len() < size {
        data.extend_from_slice(MARKER);
    }
    data.truncate(size);
    std::fs::write(&path, data).unwrap();
    path
}

fn write_pattern_file(dir: &std::path::Path, name: &str, size: usize) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, data).unwrap();
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_to_relay_8mib() {
    let (port, relay) = start_relay();
    let mut a = Node::start("A", port);
    let mut b = Node::start("B", port);

    let peer_b = wait_relay_peer(&mut a.events).await;
    let source = write_pattern_file(a.dir.path(), "relay.bin", 8 * 1024 * 1024);

    a.handle.command(Command::Send {
        peer: peer_b,
        path: source.clone(),
    });

    wait_incoming_and_accept(&mut b.events, &b.handle).await;
    let completed = wait_completed(&mut b.events).await;
    let saved = match completed {
        Event::Completed {
            path, transport, ..
        } => {
            assert_eq!(transport, droplink_core::protocol::TransportMode::Relay);
            path.unwrap()
        }
        _ => unreachable!(),
    };
    assert_eq!(
        droplink_core::hash::hash_hex(&std::fs::read(&saved).unwrap()),
        droplink_core::hash::hash_hex(&std::fs::read(&source).unwrap())
    );
    relay.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_no_plaintext_leak() {
    let (relay_port, relay) = start_relay();
    let (proxy_port, captured, proxy, _token) =
        start_proxy(relay_port, 0, CancellationToken::new()).await;

    let mut a = Node::start("A", proxy_port);
    let mut b = Node::start("B", proxy_port);
    let peer_b = wait_relay_peer(&mut a.events).await;

    let size = 8 * 1024 * 1024;
    let source = write_marker_file(a.dir.path(), size);
    a.handle.command(Command::Send {
        peer: peer_b,
        path: source,
    });
    wait_incoming_and_accept(&mut b.events, &b.handle).await;
    wait_completed(&mut b.events).await;

    let frames = captured.lock().unwrap().clone();
    let mut relayed_bytes = 0usize;
    for frame in &frames {
        let (_, payload) = decode_binary(frame).expect("relay frame");
        relayed_bytes += payload.len();
        assert!(
            !payload.windows(MARKER.len()).any(|window| window == MARKER),
            "plaintext marker leaked through the relay"
        );
    }
    assert!(
        relayed_bytes >= size,
        "expected at least {size} relayed bytes, got {relayed_bytes}"
    );

    proxy.abort();
    relay.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_drop_fails_fast() {
    let (relay_port, relay) = start_relay();
    let (proxy_port, _captured, proxy, token) =
        start_proxy(relay_port, 0, CancellationToken::new()).await;

    let mut a = Node::start("A", proxy_port);
    let mut b = Node::start("B", proxy_port);
    let peer_b = wait_relay_peer(&mut a.events).await;

    let source = write_pattern_file(a.dir.path(), "drop.bin", 64 * 1024 * 1024);
    a.handle.command(Command::Send {
        peer: peer_b,
        path: source,
    });
    wait_incoming_and_accept(&mut b.events, &b.handle).await;

    token.cancel();
    proxy.abort();
    relay.abort();

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "transfer did not fail after relay drop"
        );
        match tokio::time::timeout(remaining, a.events.recv()).await {
            Ok(Some(Event::Failed { .. })) => break,
            Ok(Some(_)) => {}
            _ => break,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_reconnect_recovers() {
    let (relay_port, relay) = start_relay();
    let token = CancellationToken::new();
    let (proxy_port, _captured, proxy, _) = start_proxy(relay_port, 0, token.clone()).await;

    let mut a = Node::start("A", proxy_port);
    let mut b = Node::start("B", proxy_port);
    let _ = wait_relay_peer(&mut a.events).await;

    // Drop every proxied connection: the engines must notice and reconnect.
    token.cancel();
    proxy.abort();
    assert!(
        wait_empty_devices(&mut a.events).await,
        "engine did not notice the relay disconnection"
    );

    let (_new_port, _, _new_proxy, _) =
        start_proxy(relay_port, proxy_port, CancellationToken::new()).await;

    let peer_b = wait_relay_peer(&mut a.events).await;
    let source = write_pattern_file(a.dir.path(), "again.bin", 512 * 1024);
    a.handle.command(Command::Send {
        peer: peer_b,
        path: source,
    });
    wait_incoming_and_accept(&mut b.events, &b.handle).await;
    wait_completed(&mut b.events).await;

    relay.abort();
}

/// WebSocket proxy that records every binary frame passing through.
async fn start_proxy(
    upstream_port: u16,
    bind_port: u16,
    token: CancellationToken,
) -> (
    u16,
    Arc<Mutex<Vec<Vec<u8>>>>,
    JoinHandle<()>,
    CancellationToken,
) {
    let listener = bind_with_retry(bind_port).await;
    let port = listener.local_addr().unwrap().port();
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_task = captured.clone();
    let task_token = token.clone();
    let task = tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = task_token.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((client, _)) = accepted else {
                break;
            };
            let captured = captured_task.clone();
            let conn_token = task_token.clone();
            tokio::spawn(async move {
                let Ok(client_ws) = tokio_tungstenite::accept_async(client).await else {
                    return;
                };
                let upstream = format!("ws://127.0.0.1:{upstream_port}/ws");
                let Ok((server_ws, _)) = connect_async(upstream).await else {
                    return;
                };
                let (mut c_sink, mut c_stream) = client_ws.split();
                let (mut s_sink, mut s_stream) = server_ws.split();
                let up_capture = captured.clone();
                let up_token = conn_token.clone();
                let up = tokio::spawn(async move {
                    loop {
                        let next = tokio::select! {
                            _ = up_token.cancelled() => break,
                            next = c_stream.next() => next,
                        };
                        let Some(Ok(message)) = next else { break };
                        if let Message::Binary(bytes) = &message {
                            up_capture.lock().unwrap().push(bytes.to_vec());
                        }
                        if s_sink.send(message).await.is_err() {
                            break;
                        }
                    }
                });
                let down = tokio::spawn(async move {
                    loop {
                        let next = tokio::select! {
                            _ = conn_token.cancelled() => break,
                            next = s_stream.next() => next,
                        };
                        let Some(Ok(message)) = next else { break };
                        if let Message::Binary(bytes) = &message {
                            captured.lock().unwrap().push(bytes.to_vec());
                        }
                        if c_sink.send(message).await.is_err() {
                            break;
                        }
                    }
                });
                let _ = tokio::join!(up, down);
            });
        }
    });
    (port, captured, task, token)
}

async fn bind_with_retry(port: u16) -> TcpListener {
    if port == 0 {
        return TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    }
    for attempt in 0..100 {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return listener,
            Err(err) => {
                eprintln!("rebind attempt {attempt} failed: {err}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    panic!("could not rebind proxy port {port}");
}
