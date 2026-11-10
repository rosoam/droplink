use droplink_core::peer::{Peer, Reach};
use droplink_net::engine::{Command, EngineConfig, Event};
use droplink_net::spawn;
use droplink_relay::room::Rooms;
use droplink_relay::server::{serve, AppState};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

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
        cfg.relay_url = Some(format!("ws://127.0.0.1:{relay_port}/ws"));
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

async fn wait_session(events: &mut mpsc::Receiver<Event>, owner: bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timeout waiting for a session");
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Some(Event::Session {
                code,
                owner: is_owner,
                ..
            })) if is_owner == owner => return code,
            Ok(Some(Event::SessionError { code, message })) => {
                panic!("session error: {code} {message}")
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("event channel closed"),
            Err(_) => panic!("timeout waiting for a session"),
        }
    }
}

async fn create_session(node: &mut Node) -> String {
    node.handle.command(Command::SessionCreate);
    wait_session(&mut node.events, true).await
}

async fn join_session(node: &mut Node, code: &str) {
    node.handle
        .command(Command::SessionJoin { code: code.into() });
    let _ = wait_session(&mut node.events, false).await;
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

fn write_pattern_file(dir: &std::path::Path, name: &str, size: usize) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, data).unwrap();
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_join_and_transfer() {
    let (port, relay) = start_relay();
    let mut a = Node::start("A", port);
    let mut b = Node::start("B", port);
    let code = create_session(&mut a).await;
    join_session(&mut b, &code).await;

    let peer_b = wait_relay_peer(&mut a.events).await;
    let source = write_pattern_file(a.dir.path(), "session.bin", 2 * 1024 * 1024);
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
async fn wrong_code_reports_no_session() {
    let (port, relay) = start_relay();
    let mut b = Node::start("B", port);
    b.handle.command(Command::SessionJoin {
        code: "ZZZZ2345".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timeout waiting for the error");
        match tokio::time::timeout(remaining, b.events.recv()).await {
            Ok(Some(Event::SessionError { code, .. })) => {
                assert_eq!(code, "no_session");
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("channel closed"),
            Err(_) => panic!("timeout"),
        }
    }
    relay.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refreshing_the_code_mid_transfer_keeps_the_transfer() {
    let (port, relay) = start_relay();
    let mut a = Node::start("A", port);
    let mut b = Node::start("B", port);
    let code = create_session(&mut a).await;
    join_session(&mut b, &code).await;
    let peer_b = wait_relay_peer(&mut a.events).await;

    let source = write_pattern_file(a.dir.path(), "refresh.bin", 8 * 1024 * 1024);
    a.handle.command(Command::Send {
        peer: peer_b,
        path: source.clone(),
    });
    wait_incoming_and_accept(&mut b.events, &b.handle).await;
    a.handle.command(Command::SessionRefresh);

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

/// Heavy smoke test: the relay path must carry 100 MiB with an identical BLAKE3.
/// Run with `cargo test -p droplink-net --test session_transfer -- --ignored`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "heavy: transfers 100 MiB through the relay"]
async fn session_transfer_100mib() {
    let (port, relay) = start_relay();
    let mut a = Node::start("A", port);
    let mut b = Node::start("B", port);
    let code = create_session(&mut a).await;
    join_session(&mut b, &code).await;
    let peer_b = wait_relay_peer(&mut a.events).await;

    let size = 100 * 1024 * 1024;
    let source = write_pattern_file(a.dir.path(), "big.bin", size);
    a.handle.command(Command::Send {
        peer: peer_b,
        path: source.clone(),
    });
    wait_incoming_and_accept(&mut b.events, &b.handle).await;
    let completed = wait_completed(&mut b.events).await;
    let saved = match completed {
        Event::Completed { path, .. } => path.unwrap(),
        _ => unreachable!(),
    };
    let sent = std::fs::read(&source).unwrap();
    let received = std::fs::read(&saved).unwrap();
    assert_eq!(received.len(), size);
    assert_eq!(
        droplink_core::hash::hash_hex(&sent),
        droplink_core::hash::hash_hex(&received)
    );
    relay.abort();
}
