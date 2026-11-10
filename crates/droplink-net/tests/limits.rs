use droplink_core::crypto::{EphemeralSecret, Session};
use droplink_core::peer::{Peer, Reach};
use droplink_core::protocol::{Control, DEFAULT_CHUNK, PROTOCOL_VERSION};
use droplink_net::engine::{Command, EngineConfig, Event};
use droplink_net::spawn;
use droplink_net::stream::{FramedStream, Incoming};
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use uuid::Uuid;

struct Node {
    handle: droplink_net::EngineHandle,
    port: u16,
    events: mpsc::Receiver<Event>,
    _dir: TempDir,
}

fn start_node(max_parallel: usize) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = EngineConfig::new(
        Uuid::new_v4().to_string(),
        "limits".into(),
        dir.path().join("recv"),
    );
    cfg.max_parallel = max_parallel;
    cfg.handshake_timeout = Duration::from_secs(2);
    let (handle, bound, events) = spawn(cfg).unwrap();
    Node {
        handle,
        port: bound.port,
        events,
        _dir: dir,
    }
}

async fn expect_busy(port: u16) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut framed = FramedStream::new(stream);
    let frame = tokio::time::timeout(Duration::from_secs(5), framed.read_frame())
        .await
        .expect("timeout waiting for busy error")
        .expect("connection closed without error frame");
    match frame {
        Incoming::Control(Control::Error { reason, .. }) => {
            assert_eq!(reason, "busy", "unexpected reason: {reason}");
        }
        other => panic!("expected busy error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_parallel_rejects_fifth() {
    let node = start_node(1);
    let _idle = TcpStream::connect(("127.0.0.1", node.port)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    expect_busy(node.port).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_ip_cap() {
    let node = start_node(4);
    let _first = TcpStream::connect(("127.0.0.1", node.port)).await.unwrap();
    let _second = TcpStream::connect(("127.0.0.1", node.port)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    expect_busy(node.port).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lying_file_size() {
    let mut node = start_node(4);
    let transfer_id = Uuid::new_v4().to_string();

    let stream = TcpStream::connect(("127.0.0.1", node.port)).await.unwrap();
    let mut framed = FramedStream::new(stream);
    let ephemeral = EphemeralSecret::generate();
    let announced = 1024 * 1024u64;
    let hello = Control::Hello {
        version: PROTOCOL_VERSION,
        transfer_id: transfer_id.clone(),
        chunk_size: DEFAULT_CHUNK,
        file_name: "liar.bin".into(),
        file_size: announced,
        eph_pub: ephemeral.public(),
        alias: "liar".into(),
    };
    framed.write_control(&hello).await.unwrap();

    let offer = framed.read_frame().await.unwrap();
    let peer_public = match offer {
        Incoming::Control(Control::KeyOffer { eph_pub }) => eph_pub,
        other => panic!("expected key offer, got {other:?}"),
    };
    let mut session = Session::sender(&hello, &ephemeral, peer_public).unwrap();

    // Accept the transfer from the UI side.
    let incoming_id = loop {
        let event = tokio::time::timeout(Duration::from_secs(5), node.events.recv())
            .await
            .unwrap()
            .unwrap();
        if let Event::Incoming(incoming) = event {
            break incoming.transfer_id;
        }
    };
    assert_eq!(incoming_id, transfer_id);
    node.handle.command(Command::Respond {
        transfer_id,
        accept: true,
    });

    let decision = framed.read_frame().await.unwrap();
    match decision {
        Incoming::SealedControl(ct) => {
            let plain = session.open(&ct).unwrap();
            assert!(matches!(
                serde_json::from_slice::<Control>(&plain).unwrap(),
                Control::Accept { .. }
            ));
        }
        other => panic!("expected sealed accept, got {other:?}"),
    }

    // Send 5 x 256 KiB while announcing 1 MiB: the fifth chunk exceeds the announced size.
    let chunk = vec![0x11u8; DEFAULT_CHUNK as usize];
    for _ in 0..5 {
        let sealed = session.seal(&chunk).unwrap();
        framed.write_data(&sealed).await.unwrap();
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "receiver did not reject oversized stream"
        );
        match tokio::time::timeout(remaining, node.events.recv()).await {
            Ok(Some(Event::Failed { reason, .. })) => {
                assert!(
                    reason.contains("more bytes"),
                    "unexpected failure reason: {reason}"
                );
                break;
            }
            Ok(Some(_)) => {}
            _ => panic!("receiver stopped without a failure event"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversize_frame_rejected_before_allocation() {
    let node = start_node(4);
    let mut stream = TcpStream::connect(("127.0.0.1", node.port)).await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // kind=DATA, version=2, len=2 MiB (above MAX_CHUNK + tag).
    let mut header = vec![1u8, 2u8];
    header.extend_from_slice(&(2u32 * 1024 * 1024).to_be_bytes());
    stream.write_all(&header).await.unwrap();

    let mut buffer = [0u8; 32];
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
        .await
        .expect("receiver did not close the connection");
    match read {
        Ok(0) => {}
        Err(_) => {}
        Ok(n) => panic!("receiver unexpectedly returned {n} bytes"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreachable_peer_fails() {
    let mut node = start_node(4);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.bin");
    std::fs::write(&path, b"x").unwrap();

    node.handle.command(Command::Send {
        peer: Peer {
            device_id: Some(Uuid::new_v4().to_string()),
            alias: "nobody".into(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 1,
            },
        },
        path,
    });

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "no failure for an unreachable peer");
        match tokio::time::timeout(remaining, node.events.recv()).await {
            Ok(Some(Event::Failed { reason, .. })) => {
                assert!(
                    reason.contains("unreachable"),
                    "unexpected reason: {reason}"
                );
                break;
            }
            Ok(Some(_)) => {}
            _ => panic!("event channel closed"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_hello_gets_error() {
    let node = start_node(4);
    let stream = TcpStream::connect(("127.0.0.1", node.port)).await.unwrap();
    let mut framed = FramedStream::new(stream);
    framed
        .write_control(&Control::Refuse {
            transfer_id: "x".into(),
            reason: "nope".into(),
        })
        .await
        .unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(5), framed.read_frame())
        .await
        .expect("timeout")
        .expect("connection closed");
    match frame {
        Incoming::Control(Control::Error { reason, .. }) => {
            assert_eq!(reason, "expected hello");
        }
        other => panic!("expected error, got {other:?}"),
    }
}
