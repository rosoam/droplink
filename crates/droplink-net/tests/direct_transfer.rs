use droplink_core::crypto::{EphemeralSecret, Session};
use droplink_core::hash::Hasher;
use droplink_core::peer::{Peer, Reach};
use droplink_core::protocol::{Control, DEFAULT_CHUNK, PROTOCOL_VERSION};
use droplink_net::engine::{Command, EngineConfig, Event};
use droplink_net::spawn;
use droplink_net::stream::{FramedStream, Incoming};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use uuid::Uuid;

const MIB: usize = 1024 * 1024;

struct Node {
    handle: droplink_net::EngineHandle,
    port: u16,
    events: mpsc::Receiver<Event>,
    dir: TempDir,
}

impl Node {
    fn start(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cfg = EngineConfig::new(
            Uuid::new_v4().to_string(),
            name.to_string(),
            dir.path().join("recv"),
        );
        let (handle, bound, events) = spawn(cfg).unwrap();
        Self {
            handle,
            port: bound.port,
            events,
            dir,
        }
    }

    fn receive_dir(&self) -> PathBuf {
        self.dir.path().join("recv")
    }

    fn peer(&self, alias: &str) -> Peer {
        Peer {
            device_id: Some(Uuid::new_v4().to_string()),
            alias: alias.to_string(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: self.port,
            },
        }
    }
}

fn write_file(dir: &Path, name: &str, size: usize) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    let mut data = vec![0u8; size];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    std::fs::write(&path, data).unwrap();
    path
}

async fn wait_for<F>(rx: &mut mpsc::Receiver<Event>, label: &str, mut pred: F) -> Event
where
    F: FnMut(&Event) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timeout waiting for {label}");
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(event)) => {
                if pred(&event) {
                    return event;
                }
            }
            Ok(None) => panic!("event channel closed while waiting for {label}"),
            Err(_) => panic!("timeout waiting for {label}"),
        }
    }
}

async fn send_and_accept(sender: &Node, receiver: &mut Node, peer: Peer, path: PathBuf) {
    sender.handle.command(Command::Send { peer, path });
    let event = wait_for(&mut receiver.events, "incoming", |e| {
        matches!(e, Event::Incoming(_))
    })
    .await;
    let incoming = match event {
        Event::Incoming(i) => i,
        _ => unreachable!(),
    };
    receiver.handle.command(Command::Respond {
        transfer_id: incoming.transfer_id,
        accept: true,
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transfer_1mib() {
    let mut receiver = Node::start("receiver");
    let mut sender = Node::start("sender");
    let source = write_file(sender.dir.path(), "payload.bin", MIB);

    let peer = receiver.peer("dest");
    send_and_accept(&sender, &mut receiver, peer, source.clone()).await;

    let completed = wait_for(&mut receiver.events, "receiver completed", |e| {
        matches!(e, Event::Completed { .. })
    })
    .await;
    let saved = match completed {
        Event::Completed { path, .. } => path.unwrap(),
        _ => unreachable!(),
    };
    assert_eq!(
        std::fs::read(&saved).unwrap(),
        std::fs::read(&source).unwrap()
    );

    wait_for(&mut sender.events, "sender completed", |e| {
        matches!(e, Event::Completed { .. })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transfer_64mib() {
    let mut receiver = Node::start("receiver");
    let sender = Node::start("sender");
    let source = write_file(sender.dir.path(), "big.bin", 64 * MIB);

    let peer = receiver.peer("dest");
    send_and_accept(&sender, &mut receiver, peer, source.clone()).await;

    let completed = wait_for(&mut receiver.events, "receiver completed", |e| {
        matches!(e, Event::Completed { .. })
    })
    .await;
    let saved = match completed {
        Event::Completed { path, .. } => path.unwrap(),
        _ => unreachable!(),
    };
    assert_eq!(std::fs::metadata(&saved).unwrap().len(), (64 * MIB) as u64);
    assert_eq!(
        droplink_core::hash::hash_hex(&std::fs::read(&saved).unwrap()),
        droplink_core::hash::hash_hex(&std::fs::read(&source).unwrap())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuse_leaves_no_file_and_keeps_source() {
    let mut receiver = Node::start("receiver");
    let mut sender = Node::start("sender");
    let source = write_file(sender.dir.path(), "refused.bin", 64 * 1024);

    sender.handle.command(Command::Send {
        peer: receiver.peer("dest"),
        path: source.clone(),
    });

    let event = wait_for(&mut receiver.events, "incoming", |e| {
        matches!(e, Event::Incoming(_))
    })
    .await;
    let incoming = match event {
        Event::Incoming(i) => i,
        _ => unreachable!(),
    };
    receiver.handle.command(Command::Respond {
        transfer_id: incoming.transfer_id,
        accept: false,
    });

    wait_for(&mut sender.events, "sender failed", |e| {
        matches!(e, Event::Failed { .. })
    })
    .await;
    wait_for(&mut receiver.events, "receiver failed", |e| {
        matches!(e, Event::Failed { .. })
    })
    .await;

    assert!(source.exists(), "sender must never delete the source file");
    assert!(
        std::fs::read_dir(receiver.receive_dir())
            .map(|d| d.count())
            .unwrap_or(0)
            == 0
    );
}

// ---------------------------------------------------------------------------------------------
// Fake peer helpers: exercise the receiver/sender invariants that a cooperative peer cannot.
// ---------------------------------------------------------------------------------------------

struct FakeSender {
    framed: FramedStream<TcpStream>,
    session: Session,
    transfer_id: String,
}

impl FakeSender {
    async fn connect(port: u16, size: u64) -> Self {
        let transfer_id = Uuid::new_v4().to_string();
        let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut framed = FramedStream::new(stream);
        let ephemeral = EphemeralSecret::generate();
        let hello = Control::Hello {
            version: PROTOCOL_VERSION,
            transfer_id: transfer_id.clone(),
            chunk_size: DEFAULT_CHUNK,
            file_name: "fake.bin".into(),
            file_size: size,
            eph_pub: ephemeral.public(),
            alias: "fake".into(),
        };
        framed.write_control(&hello).await.unwrap();
        let offer = framed.read_frame().await.unwrap();
        let peer_public = match offer {
            Incoming::Control(Control::KeyOffer { eph_pub }) => eph_pub,
            other => panic!("expected key offer, got {other:?}"),
        };
        let session = Session::sender(&hello, &ephemeral, peer_public).unwrap();
        Self {
            framed,
            session,
            transfer_id,
        }
    }

    async fn wait_accept(&mut self) {
        let frame = self.framed.read_frame().await.unwrap();
        let plain = match frame {
            Incoming::SealedControl(ct) => self.session.open(&ct).unwrap(),
            other => panic!("expected sealed accept, got {other:?}"),
        };
        match serde_json::from_slice::<Control>(&plain).unwrap() {
            Control::Accept { .. } => {}
            other => panic!("expected accept, got {other:?}"),
        }
    }

    async fn send_data(&mut self, data: &[u8]) {
        let sealed = self.session.seal(data).unwrap();
        self.framed.write_data(&sealed).await.unwrap();
    }

    async fn send_corrupt_data(&mut self, data: &[u8]) {
        let mut sealed = self.session.seal(data).unwrap();
        sealed[0] ^= 0xFF;
        self.framed.write_data(&sealed).await.unwrap();
    }

    async fn send_end(&mut self, hash: [u8; 32]) {
        let sealed = self
            .session
            .seal(
                &serde_json::to_vec(&Control::End {
                    transfer_id: self.transfer_id.clone(),
                    hash,
                })
                .unwrap(),
            )
            .unwrap();
        self.framed.write_sealed_control(&sealed).await.unwrap();
    }
}

async fn respond_after_incoming(node: &mut Node, transfer_id: &str, accept: bool) {
    let event = wait_for(
        &mut node.events,
        "incoming",
        |e| matches!(e, Event::Incoming(i) if i.transfer_id == transfer_id),
    )
    .await;
    drop(event);
    node.handle.command(Command::Respond {
        transfer_id: transfer_id.to_string(),
        accept,
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hash_mismatch_is_rejected() {
    let mut receiver = Node::start("receiver");
    let mut fake = FakeSender::connect(receiver.port, 6).await;
    let transfer_id = fake.transfer_id.clone();

    respond_after_incoming(&mut receiver, &transfer_id, true).await;
    fake.wait_accept().await;
    fake.send_data(b"abc").await;
    fake.send_end([0xAB; 32]).await;

    let failed = wait_for(&mut receiver.events, "receiver failed", |e| {
        matches!(e, Event::Failed { .. })
    })
    .await;
    match failed {
        Event::Failed { reason, .. } => assert!(
            reason.to_lowercase().contains("hash"),
            "expected hash failure, got {reason}"
        ),
        _ => unreachable!(),
    }
    assert_no_received_files(&receiver);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tampered_ciphertext_is_rejected() {
    let mut receiver = Node::start("receiver");
    let mut fake = FakeSender::connect(receiver.port, 6).await;
    let transfer_id = fake.transfer_id.clone();

    respond_after_incoming(&mut receiver, &transfer_id, true).await;
    fake.wait_accept().await;
    fake.send_corrupt_data(b"abc").await;

    let failed = wait_for(&mut receiver.events, "receiver failed", |e| {
        matches!(e, Event::Failed { .. })
    })
    .await;
    match failed {
        Event::Failed { reason, .. } => assert!(
            reason.to_lowercase().contains("crypto")
                || reason.to_lowercase().contains("authentication"),
            "expected crypto failure, got {reason}"
        ),
        _ => unreachable!(),
    }
    assert_no_received_files(&receiver);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn truncated_stream_is_rejected() {
    let mut receiver = Node::start("receiver");
    let mut fake = FakeSender::connect(receiver.port, 6).await;
    let transfer_id = fake.transfer_id.clone();

    respond_after_incoming(&mut receiver, &transfer_id, true).await;
    fake.wait_accept().await;
    fake.send_data(b"abc").await;
    drop(fake);

    wait_for(&mut receiver.events, "receiver failed", |e| {
        matches!(e, Event::Failed { .. })
    })
    .await;
    assert_no_received_files(&receiver);
}

fn assert_no_received_files(node: &Node) {
    let dir = node.receive_dir();
    let count = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().is_file())
                .count()
        })
        .unwrap_or(0);
    assert_eq!(count, 0, "no file (final or .part) must remain in {dir:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_completed_without_done_ok() {
    let mut sender = Node::start("sender");
    let source = write_file(sender.dir.path(), "x.bin", 32 * 1024);

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut framed = FramedStream::new(stream);
        let hello_frame = framed.read_frame().await.unwrap();
        let hello = match hello_frame {
            Incoming::Control(control @ Control::Hello { .. }) => control,
            other => panic!("expected hello, got {other:?}"),
        };
        let transfer_id = match &hello {
            Control::Hello { transfer_id, .. } => transfer_id.clone(),
            _ => unreachable!(),
        };
        let ephemeral = EphemeralSecret::generate();
        framed
            .write_control(&Control::KeyOffer {
                eph_pub: ephemeral.public(),
            })
            .await
            .unwrap();
        let mut session = Session::receiver(&hello, &ephemeral).unwrap();
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
                    session.open(&ct).unwrap();
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
                    ok: false,
                    reason: Some("simulated corruption".into()),
                })
                .unwrap(),
            )
            .unwrap();
        framed.write_sealed_control(&done).await.unwrap();
    });

    sender.handle.command(Command::Send {
        peer: Peer {
            device_id: Some(Uuid::new_v4().to_string()),
            alias: "fake".into(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port,
            },
        },
        path: source,
    });

    wait_for(&mut sender.events, "sender failed", |e| {
        matches!(e, Event::Failed { .. })
    })
    .await;
    fake.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_same_name_concurrent_produce_two_files() {
    let mut receiver = Node::start("receiver");
    let sender_a = Node::start("a");
    let sender_b = Node::start("b");
    let path_a = write_file(sender_a.dir.path(), "same.txt", 32 * 1024);
    let path_b = write_file(sender_b.dir.path(), "same.txt", 48 * 1024);

    sender_a.handle.command(Command::Send {
        peer: receiver.peer("a"),
        path: path_a,
    });
    let first = wait_for(&mut receiver.events, "incoming 1", |e| {
        matches!(e, Event::Incoming(_))
    })
    .await;
    if let Event::Incoming(i) = first {
        receiver.handle.command(Command::Respond {
            transfer_id: i.transfer_id,
            accept: true,
        });
    }

    sender_b.handle.command(Command::Send {
        peer: receiver.peer("b"),
        path: path_b,
    });

    let mut completed = 0;
    while completed < 2 {
        let event = wait_for(&mut receiver.events, "incoming or completed", |e| {
            matches!(e, Event::Incoming(_) | Event::Completed { .. })
        })
        .await;
        match event {
            Event::Incoming(i) => receiver.handle.command(Command::Respond {
                transfer_id: i.transfer_id,
                accept: true,
            }),
            Event::Completed { .. } => completed += 1,
            _ => {}
        }
    }

    let files: Vec<_> = std::fs::read_dir(receiver.receive_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("same")
        })
        .collect();
    assert_eq!(files.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_aborts_transfer() {
    let mut receiver = Node::start("receiver");
    let mut sender = Node::start("sender");
    let source = write_file(sender.dir.path(), "slow.bin", 32 * MIB);

    sender.handle.command(Command::Send {
        peer: receiver.peer("dest"),
        path: source,
    });

    let queued = wait_for(&mut sender.events, "queued", |e| {
        matches!(e, Event::Queued { .. })
    })
    .await;
    let transfer_id = match queued {
        Event::Queued { transfer_id, .. } => transfer_id,
        _ => unreachable!(),
    };

    let event = wait_for(&mut receiver.events, "incoming", |e| {
        matches!(e, Event::Incoming(_))
    })
    .await;
    if let Event::Incoming(i) = event {
        receiver.handle.command(Command::Respond {
            transfer_id: i.transfer_id,
            accept: true,
        });
    }
    sender.handle.command(Command::Cancel { transfer_id });

    wait_for(&mut sender.events, "sender failed", |e| {
        matches!(e, Event::Failed { .. })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_device_serialized() {
    let mut receiver = Node::start("receiver");
    let sender = Node::start("sender");
    let peer = receiver.peer("dest");

    for i in 0..3 {
        let path = write_file(sender.dir.path(), &format!("f{i}.bin"), 256 * 1024);
        sender.handle.command(Command::Send {
            peer: peer.clone(),
            path,
        });
    }

    let mut active = 0i32;
    let mut max_active = 0i32;
    let mut completed = 0;
    while completed < 3 {
        let event = wait_for(&mut receiver.events, "transfer event", |e| {
            matches!(
                e,
                Event::Incoming(_) | Event::Completed { .. } | Event::Failed { .. }
            )
        })
        .await;
        match event {
            Event::Incoming(i) => {
                active += 1;
                max_active = max_active.max(active);
                receiver.handle.command(Command::Respond {
                    transfer_id: i.transfer_id,
                    accept: true,
                });
            }
            Event::Completed { .. } => {
                active -= 1;
                completed += 1;
            }
            Event::Failed { .. } => {
                active -= 1;
                completed += 1;
            }
            _ => {}
        }
    }
    assert_eq!(max_active, 1, "transfers to one device must be serialized");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn progress_rate_capped() {
    let mut receiver = Node::start("receiver");
    let sender = Node::start("sender");
    let source = write_file(sender.dir.path(), "big.bin", 64 * MIB);

    let started = Instant::now();
    sender.handle.command(Command::Send {
        peer: receiver.peer("dest"),
        path: source,
    });
    let event = wait_for(&mut receiver.events, "incoming", |e| {
        matches!(e, Event::Incoming(_))
    })
    .await;
    if let Event::Incoming(i) = event {
        receiver.handle.command(Command::Respond {
            transfer_id: i.transfer_id,
            accept: true,
        });
    }

    let mut progress = 0usize;
    loop {
        let event = wait_for(&mut receiver.events, "progress or completed", |e| {
            matches!(e, Event::Progress { .. } | Event::Completed { .. })
        })
        .await;
        match event {
            Event::Progress { .. } => progress += 1,
            Event::Completed { .. } => break,
            _ => {}
        }
    }
    let elapsed = started.elapsed().as_secs_f64().max(0.1);
    let rate = progress as f64 / elapsed;
    assert!(
        rate <= 25.0,
        "progress rate {rate:.1}/s exceeds the 20/s budget (elapsed {elapsed:.2}s)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_consumer_does_not_stall_transfer() {
    let sender = Node::start("sender");
    let source = write_file(sender.dir.path(), "x.bin", 8 * MIB);

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut framed = FramedStream::new(stream);
        let hello_frame = framed.read_frame().await.unwrap();
        let hello = match hello_frame {
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
            .seal(&serde_json::to_vec(&Control::Accept { transfer_id }).unwrap())
            .unwrap();
        framed.write_sealed_control(&accept).await.unwrap();

        let mut received = 0usize;
        let mut hasher = Hasher::new();
        loop {
            match framed.read_frame().await.unwrap() {
                Incoming::Data(ct) => {
                    let plain = session.open(&ct).unwrap();
                    hasher.update(&plain);
                    received += plain.len();
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
        (received, hasher.finalize())
    });

    // Never read `sender.events`: a stalled consumer must not slow the transfer down.
    sender.handle.command(Command::Send {
        peer: Peer {
            device_id: Some(Uuid::new_v4().to_string()),
            alias: "fake".into(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port,
            },
        },
        path: source.clone(),
    });

    let (received, hash) = tokio::time::timeout(Duration::from_secs(60), fake)
        .await
        .expect("transfer stalled because the event consumer was not draining")
        .unwrap();
    assert_eq!(received, 8 * MIB);
    assert_eq!(
        hash,
        droplink_core::hash::hash_bytes(&std::fs::read(&source).unwrap())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bind_once_gives_distinct_ports() {
    let a = Node::start("a");
    let b = Node::start("b");
    assert_ne!(a.port, b.port);
    assert!(a.port > 0 && b.port > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_read_proof() {
    let mut receiver = Node::start("receiver");
    let sender = Node::start("sender");
    let source = write_file(sender.dir.path(), "once.bin", MIB);

    let before = droplink_net::sender::opens_for(&source);
    let peer = receiver.peer("dest");
    send_and_accept(&sender, &mut receiver, peer, source.clone()).await;
    wait_for(&mut receiver.events, "completed", |e| {
        matches!(e, Event::Completed { .. })
    })
    .await;
    let after = droplink_net::sender::opens_for(&source);
    assert_eq!(
        after - before,
        1,
        "sender must open the source exactly once"
    );
}
