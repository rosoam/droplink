use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use droplink_core::hash::Hasher;
use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::time::Instant;

const DATA_LEN: usize = 32 * 1024 * 1024;

fn throughput_mbps(runs: usize, mut f: impl FnMut(&[u8])) -> f64 {
    let data = vec![0x5Au8; DATA_LEN];
    let mut best = f64::INFINITY;
    for _ in 0..runs {
        let start = Instant::now();
        f(black_box(&data));
        let secs = start.elapsed().as_secs_f64();
        if secs < best {
            best = secs;
        }
    }
    (DATA_LEN as f64 / (1024.0 * 1024.0)) / best
}

fn bench_hash(c: &mut Criterion) {
    let blake3_mbps = throughput_mbps(3, |data| {
        let mut h = Hasher::new();
        h.update(data);
        black_box(h.finalize());
    });
    let sha256_mbps = throughput_mbps(3, |data| {
        let mut h = Sha256::new();
        h.update(data);
        black_box(h.finalize());
    });
    let ratio = blake3_mbps / sha256_mbps;
    println!(
        "blake3: {blake3_mbps:.0} MiB/s | sha256: {sha256_mbps:.0} MiB/s | ratio: {ratio:.2}x"
    );
    assert!(
        ratio >= 3.0,
        "BLAKE3 must stay faster than SHA-256 (got {ratio:.2}x)"
    );

    let data = vec![0x5Au8; DATA_LEN];
    let mut group = c.benchmark_group("hash_32m");
    group.throughput(Throughput::Bytes(DATA_LEN as u64));
    group.bench_function("blake3", |b| {
        b.iter(|| {
            let mut h = Hasher::new();
            h.update(black_box(&data));
            black_box(h.finalize());
        })
    });
    group.bench_function("sha256", |b| {
        b.iter(|| {
            let mut h = Sha256::new();
            h.update(black_box(&data));
            black_box(h.finalize());
        })
    });
    group.finish();
}

criterion_group!(benches, bench_hash);
criterion_main!(benches);
