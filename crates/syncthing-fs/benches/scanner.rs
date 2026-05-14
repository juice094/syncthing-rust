use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::io::Write;
use syncthing_fs::scanner;
use tempfile::NamedTempFile;

fn bench_scan_small_file(c: &mut Criterion) {
    let mut tmp = NamedTempFile::new().unwrap();
    tmp.write_all(&[0u8; 1024]).unwrap();
    let path = tmp.path().to_path_buf();
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("scan_file_1kb", |b| {
        b.iter(|| {
            let _ = rt.block_on(async { scanner::scan_file(black_box(&path), 128 * 1024).await });
        });
    });
}

fn bench_scan_medium_file(c: &mut Criterion) {
    let mut tmp = NamedTempFile::new().unwrap();
    tmp.write_all(&[0u8; 1024 * 1024]).unwrap();
    let path = tmp.path().to_path_buf();
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("scan_file_1mb", |b| {
        b.iter(|| {
            let _ = rt.block_on(async { scanner::scan_file(black_box(&path), 128 * 1024).await });
        });
    });
}

criterion_group!(benches, bench_scan_small_file, bench_scan_medium_file);
criterion_main!(benches);
