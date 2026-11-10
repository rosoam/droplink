use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use droplink_core::frame::{decode, encode_control, encode_data, Frame};
use droplink_core::protocol::{Control, DEFAULT_CHUNK, PROTOCOL_VERSION};

fn bench_frame(c: &mut Criterion) {
    let payload = vec![0xA5u8; DEFAULT_CHUNK as usize];
    let hello = Control::Hello {
        version: PROTOCOL_VERSION,
        transfer_id: "11111111-1111-4111-8111-111111111111".to_string(),
        chunk_size: DEFAULT_CHUNK,
        file_name: "bench.bin".to_string(),
        file_size: 32 * 1024 * 1024,
        eph_pub: [1u8; 32],
        alias: "bench".to_string(),
    };

    let mut group = c.benchmark_group("frame");
    group.throughput(Throughput::Bytes(payload.len() as u64));

    group.bench_function("encode_data_256k", |b| {
        let mut buf = Vec::with_capacity(payload.len() + 16);
        b.iter(|| {
            buf.clear();
            encode_data(&mut buf, black_box(&payload)).unwrap();
        })
    });

    let mut encoded = Vec::new();
    encode_data(&mut encoded, &payload).unwrap();
    group.bench_function("decode_data_256k", |b| {
        b.iter(|| {
            let (_frame, _n) = decode(black_box(&encoded)).unwrap().unwrap();
            assert!(matches!(_frame, Frame::Data(_)));
        })
    });

    group.bench_function("encode_control_hello", |b| {
        let mut buf = Vec::with_capacity(256);
        b.iter(|| {
            buf.clear();
            encode_control(&mut buf, black_box(&hello)).unwrap();
        })
    });

    group.finish();
}

criterion_group!(benches, bench_frame);
criterion_main!(benches);
