//! BEP encode/decode benchmarks (TUNING_PLAN T-A1)
//!
//! 运行：cargo bench -p bep-protocol
//! 报告：target/criterion/

use bep_protocol::messages::Hello;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn bench_hello_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("hello_encode");

    let hello = Hello::new(
        "device-name-32-bytes-padding-xxxx",
        "syncthing-rust",
        "0.2.0-beta",
    );

    group.bench_function("encode_to_vec", |b| {
        b.iter(|| {
            let v = black_box(&hello).encode_to_vec();
            black_box(v);
        });
    });

    group.finish();
}

fn bench_hello_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("hello_decode");

    let hello = Hello::new(
        "device-name-32-bytes-padding-xxxx",
        "syncthing-rust",
        "0.2.0-beta",
    );
    let buf = hello.encode_to_vec();
    group.throughput(Throughput::Bytes(buf.len() as u64));

    group.bench_function("decode", |b| {
        b.iter(|| {
            let h = Hello::decode(black_box(&buf)).expect("decode hello");
            black_box(h);
        });
    });

    group.finish();
}

fn bench_hello_roundtrip_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("hello_roundtrip_by_size");

    for name_len in [16usize, 64, 256, 1024].iter() {
        let device_name = "x".repeat(*name_len);
        let hello = Hello::new(device_name.as_str(), "syncthing-rust", "0.2.0-beta");
        let encoded = hello.encode_to_vec();
        group.throughput(Throughput::Bytes(encoded.len() as u64));

        group.bench_with_input(BenchmarkId::from_parameter(name_len), name_len, |b, _| {
            b.iter(|| {
                let v = black_box(&hello).encode_to_vec();
                let decoded = Hello::decode(&v).expect("decode");
                black_box(decoded);
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_hello_encode,
    bench_hello_decode,
    bench_hello_roundtrip_sizes
);
criterion_main!(benches);
