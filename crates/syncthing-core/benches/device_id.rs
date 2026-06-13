use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use syncthing_core::DeviceId;

fn bench_device_id_parse(c: &mut Criterion) {
    let id_str = "XCBFBGS-S4OBNCB-NNACTKO-UJX7V7W-GZLEN65-4N6W4JS-OKDNJBL-EOQXHQ7";
    c.bench_function("device_id_parse_base32", |b| {
        b.iter(|| {
            let _ = id_str.parse::<DeviceId>().unwrap();
        });
    });
}

fn bench_device_id_to_string(c: &mut Criterion) {
    let bytes = [0u8; 32];
    let id = DeviceId::from_bytes_array(bytes);
    c.bench_function("device_id_to_string_base32", |b| {
        b.iter(|| {
            let _ = black_box(id.to_string());
        });
    });
}

fn bench_device_id_roundtrip(c: &mut Criterion) {
    let bytes = [0u8; 32];
    let id = DeviceId::from_bytes_array(bytes);
    c.bench_function("device_id_roundtrip", |b| {
        b.iter(|| {
            let s = id.to_string();
            let _ = s.parse::<DeviceId>().unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_device_id_parse,
    bench_device_id_to_string,
    bench_device_id_roundtrip
);
criterion_main!(benches);
