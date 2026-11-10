//! Backpressure: a slow receiver must slow the sender down instead of letting it buffer the whole file.

use droplink_core::crypto::{EphemeralSecret, Session};
use droplink_core::peer::{Peer, Reach};
use droplink_core::protocol::Control;
use droplink_net::engine::{Command, EngineConfig, Event};
use droplink_net::spawn;
use droplink_net::stream::{FramedStream, Incoming};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use uuid::Uuid;

const SIZE: usize = 4 * 1024 * 1024;
const MAX_IN_FLIGHT: u64 = 4 * 1024 * 1024;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_reader() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = EngineConfig::new(
        Uuid::new_v4().to_string(),
        "slow".into(),
        dir.path().join("recv"),
    );
    let (handle, _bound, mut events) = spawn(cfg).unwrap();

    // Source file owned by the sender engine.
    let sender_dir = tempfile::tempdir().unwrap();
    std::fs::write(sender_dir.path().join("slow.bin"), vec![0xA7u8; SIZE]).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(AtomicU64::new(0));
    let received_task = received.clone();

    let fake = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut framed = FramedStream::new(stream);
        let hello = match framed.read_frame().await.unwrap() {
            Incoming::Control(control @ Control::Hello { .. }) => control,
            other => panic!("expected hello, got {other:?}"),
        };
        let ephemeral = EphemeralSecret::generate();
        framed
            .write_control(&Control::KeyOffer {
                eph_pub: ephemeral.public(),
            })
            .await
            .unwrap();
        let mut session = Session::receiver(&hello, &ephemeral).unwrap();
        let transfer_id = match &hello {
            Control::Hello { transfer_id, .. } => transfer_id.clone(),
            _ => unreachable!(),
        };
        let accept = session
            .seal(
                &serde_json::to_vec(&Control::Accept {
                    transfer_id: transfer_id.clone(),
                })
                .unwrap(),
            )
            .unwrap();
        framed.write_sealed_control(&accept).await.unwrap();

        loop {
            match framed.read_frame().await.unwrap() {
                Incoming::Data(ct) => {
                    let plain = session.open(&ct).unwrap();
                    received_task.fetch_add(plain.len() as u64, Ordering::SeqCst);
                    // ~1 MiB/s with 256 KiB chunks.
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Incoming::SealedControl(ct) => {
                    let plain = session.open(&ct).unwrap();
                    if matches!(
                        serde_json::from_slice::<Control>(&plain).unwrap(),
                        Control::End { .. }
                    ) {
                        break;
                    }
                }
                _ => {}
            }
        }
        let done = session
            .seal(
                &serde_json::to_vec(&Control::Done {
                    transfer_id,
                    ok: true,
                    reason: None,
                })
                .unwrap(),
            )
            .unwrap();
        framed.write_sealed_control(&done).await.unwrap();
    });

    handle.command(Command::Send {
        peer: Peer {
            device_id: Some(Uuid::new_v4().to_string()),
            alias: "slow".into(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port,
            },
        },
        path: sender_dir.path().join("slow.bin"),
    });

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "transfer did not complete in time");
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Some(Event::Progress { bytes, .. })) => {
                let consumed = received.load(Ordering::SeqCst);
                assert!(
                    bytes <= consumed + MAX_IN_FLIGHT,
                    "sender buffered {bytes} bytes while the receiver consumed only {consumed}"
                );
            }
            Ok(Some(Event::Completed { .. })) => break,
            Ok(Some(Event::Failed { reason, .. })) => panic!("transfer failed: {reason}"),
            Ok(Some(_)) => {}
            Ok(None) => panic!("event channel closed"),
            Err(_) => panic!("timeout"),
        }
    }

    fake.await.unwrap();
    assert_eq!(received.load(Ordering::SeqCst), SIZE as u64);
    let _ = dir;
}
