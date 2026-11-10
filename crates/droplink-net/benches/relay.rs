//! Relay throughput bench. The meaningful end-to-end measurement lands with the relay transport
//! (S08); this keeps the target wired and measures the relay frame codec in the meantime.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use droplink_core::relay::{decode_binary, encode_binary};

const CIRCUIT: &str = "33333333-3333-4333-8333-333333333333";

fn bench_relay(c: &mut Criterion) {
    let payload = vec![0x5Au8; 256 * 1024];
    let frame = encode_binary(CIRCUIT, &payload).unwrap();

    let mut group = c.benchmark_group("relay");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("encode_binary_256k", |b| {
        b.iter(|| encode_binary(CIRCUIT, &payload).unwrap())
    });
    group.bench_function("decode_binary_256k", |b| {
        b.iter(|| decode_binary(&frame).unwrap())
    });
    group.finish();
}

criterion_group!(benches, bench_relay);
criterion_main!(benches);
