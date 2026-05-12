//! T-B1: Serial vs Parallel SHA-256 block hashing benchmark
//!
//! Validates the rayon parallelization gain on large multi-block files.
//! 运行: cargo bench -p syncthing-fs --bench hash_parallel
//! 报告: target/criterion/

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

const BLOCK_SIZE: usize = 128 * 1024; // Standard 128 KiB block

/// Single-threaded SHA-256 hashing (baseline for parallel comparison)
fn hash_blocks_serial(data: &[u8], block_size: usize) -> Vec<Vec<u8>> {
    data.chunks(block_size)
        .map(|chunk| {
            let mut hasher = Sha256::new();
            hasher.update(chunk);
            hasher.finalize().to_vec()
        })
        .collect()
}

/// Rayon-parallel SHA-256 hashing (mirror of scanner::hash implementation)
fn hash_blocks_parallel(data: &[u8], block_size: usize) -> Vec<Vec<u8>> {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    let pool = POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_cpus::get().max(2))
            .thread_name(|i| format!("hash-bench-{}", i))
            .build()
            .expect("bench thread pool")
    });

    pool.install(|| {
        data.par_chunks(block_size)
            .map(|chunk| {
                let mut hasher = Sha256::new();
                hasher.update(chunk);
                hasher.finalize().to_vec()
            })
            .collect()
    })
}

fn bench_serial_vs_parallel(c: &mut Criterion) {
    // 测试不同大小的数据，从小到大评估并行收益
    // 小数据（<1 MB）：并行可能没有收益（spawn 开销 > 计算）
    // 大数据（>16 MB）：并行应有明显收益（数百 blocks）
    let sizes: Vec<(usize, &str)> = vec![
        (1 * 1024 * 1024, "1MiB"),    // 8 blocks
        (16 * 1024 * 1024, "16MiB"),  // 128 blocks
        (64 * 1024 * 1024, "64MiB"),  // 512 blocks
        (256 * 1024 * 1024, "256MiB"), // 2048 blocks
    ];

    for (size, label) in &sizes {
        // 准备测试数据（伪随机，避免 SHA 缓存命中）
        let data: Vec<u8> = (0..*size).map(|i| (i as u8).wrapping_mul(31)).collect();

        let mut group = c.benchmark_group(format!("hash_{}", label));
        group.throughput(Throughput::Bytes(*size as u64));

        group.bench_function(BenchmarkId::new("serial", label), |b| {
            b.iter(|| {
                let hashes = hash_blocks_serial(black_box(&data), BLOCK_SIZE);
                black_box(hashes);
            });
        });

        group.bench_function(BenchmarkId::new("parallel", label), |b| {
            b.iter(|| {
                let hashes = hash_blocks_parallel(black_box(&data), BLOCK_SIZE);
                black_box(hashes);
            });
        });

        group.finish();
    }
}

criterion_group!(benches, bench_serial_vs_parallel);
criterion_main!(benches);
