use criterion::{criterion_group, criterion_main, Criterion};
use sha2::{Digest, Sha256};
use std::hint::black_box;

fn bench_sha256_single_block(c: &mut Criterion) {
    let data = vec![0u8; 128 * 1024];
    c.bench_function("sha256_128kb_single_thread", |b| {
        b.iter(|| {
            let mut hasher = Sha256::new();
            hasher.update(black_box(&data));
            let _ = hasher.finalize();
        });
    });
}

fn bench_sha256_1mb(c: &mut Criterion) {
    let data = vec![0u8; 1024 * 1024];
    c.bench_function("sha256_1mb_single_thread", |b| {
        b.iter(|| {
            let mut hasher = Sha256::new();
            hasher.update(black_box(&data));
            let _ = hasher.finalize();
        });
    });
}

criterion_group!(benches, bench_sha256_single_block, bench_sha256_1mb);
criterion_main!(benches);
