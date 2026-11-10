use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use droplink_core::peer::{Peer, Reach};
use droplink_net::engine::{Command, EngineConfig, Event};
use droplink_net::spawn;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use uuid::Uuid;

const SIZE: usize = 256 * 1024 * 1024;
const TARGET_MBPS: f64 = 250.0;

struct Pair {
    sender_cmd: droplink_net::EngineHandle,
    receiver_cmd: droplink_net::EngineHandle,
    receiver_events: tokio::sync::mpsc::Receiver<Event>,
    peer: Peer,
    source: PathBuf,
    _sender_dir: TempDir,
    _receiver_dir: TempDir,
}

fn setup() -> Pair {
    let sender_dir = tempfile::tempdir().unwrap();
    let receiver_dir = tempfile::tempdir().unwrap();
    let source = sender_dir.path().join("bench.bin");
    let data = vec![0xA5u8; SIZE];
    std::fs::write(&source, data).unwrap();

    let (sender_cmd, _, _sender_events) = spawn(EngineConfig::new(
        Uuid::new_v4().to_string(),
        "sender".into(),
        sender_dir.path().join("recv"),
    ))
    .unwrap();
    let (receiver_cmd, bound, receiver_events) = spawn(EngineConfig::new(
        Uuid::new_v4().to_string(),
        "receiver".into(),
        receiver_dir.path().join("recv"),
    ))
    .unwrap();

    let peer = Peer {
        device_id: Some(Uuid::new_v4().to_string()),
        alias: "receiver".into(),
        reach: Reach::Direct {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: bound.port,
        },
    };

    Pair {
        sender_cmd,
        receiver_cmd,
        receiver_events,
        peer,
        source,
        _sender_dir: sender_dir,
        _receiver_dir: receiver_dir,
    }
}

async fn run_once(pair: &mut Pair) -> Duration {
    let started = Instant::now();
    pair.sender_cmd.command(Command::Send {
        peer: pair.peer.clone(),
        path: pair.source.clone(),
    });

    loop {
        let event = tokio::time::timeout(Duration::from_secs(120), pair.receiver_events.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        match event {
            Event::Incoming(incoming) => pair.receiver_cmd.command(Command::Respond {
                transfer_id: incoming.transfer_id,
                accept: true,
            }),
            Event::Completed { .. } => break,
            Event::Failed { reason, .. } => panic!("transfer failed: {reason}"),
            _ => {}
        }
    }
    started.elapsed()
}

fn measure(rt: &tokio::runtime::Runtime) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..3 {
        let mut pair = rt.block_on(async { setup() });
        let elapsed = rt.block_on(async { run_once(&mut pair).await });
        let mbps = (SIZE as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();
        if mbps < best {
            best = mbps;
        }
    }
    best
}

fn bench_loopback(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mbps = measure(&rt);
    println!("loopback: {mbps:.0} MB/s (target {TARGET_MBPS:.0} MB/s)");
    assert!(
        mbps >= TARGET_MBPS,
        "loopback throughput {mbps:.0} MB/s is below the {TARGET_MBPS:.0} MB/s budget"
    );

    let mut group = c.benchmark_group("loopback");
    group.throughput(Throughput::Bytes(SIZE as u64));
    group.sample_size(10);
    group.bench_function("transfer_256mib", |b| {
        b.iter(|| {
            let mut pair = rt.block_on(async { setup() });
            rt.block_on(async { run_once(&mut pair).await })
        })
    });
    group.finish();
}

criterion_group!(benches, bench_loopback);
criterion_main!(benches);
