//! Scanner benchmarks (TUNING_PLAN T-A1)
//!
//! 运行：cargo bench -p syncthing-fs
//! 报告：target/criterion/

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::path::PathBuf;

fn bench_scan_file(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scan_file_by_size");

    let sizes: Vec<usize> = vec![1024, 64 * 1024, 1024 * 1024, 16 * 1024 * 1024];

    for size in &sizes {
        let tmp = tempfile::tempdir().unwrap();
        let path: PathBuf = tmp.path().join(format!("file_{}.bin", size));
        std::fs::write(&path, vec![0u8; *size]).unwrap();
        group.throughput(Throughput::Bytes(*size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.to_async(&rt).iter(|| async {
                let info = syncthing_fs::scanner::scan_file(black_box(&path), 128 * 1024)
                    .await
                    .expect("scan file");
                black_box(info);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_scan_file);
criterion_main!(benches);
