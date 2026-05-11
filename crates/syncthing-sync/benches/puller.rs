//! Puller benchmarks (TUNING_PLAN T-A1)
//!
//! 运行：cargo bench -p syncthing-sync
//! 报告：target/criterion/

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use syncthing_sync::puller::{BlockSource, Puller};
use syncthing_sync::database::MemoryDatabase;
use syncthing_sync::events::EventPublisher;
use syncthing_core::types::{BlockInfo, FileInfo, Folder};
use bytes::Bytes;
use std::sync::Arc;

struct ZeroBlockSource;

#[async_trait::async_trait]
impl BlockSource for ZeroBlockSource {
    async fn request_block(
        &self,
        _folder: &str,
        _file: &str,
        block: &BlockInfo,
        _block_no: usize,
    ) -> syncthing_sync::error::Result<Bytes> {
        Ok(Bytes::from(vec![0u8; block.size as usize]))
    }
}

fn bench_pull_single_file(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("pull_single_file_by_size");

    let sizes: Vec<usize> = vec![1024, 1024 * 1024, 16 * 1024 * 1024];

    for size in &sizes {
        let tmp = tempfile::tempdir().unwrap();
        let folder = Folder::new("bench", tmp.path().to_str().unwrap());
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(1);
        let puller = Puller::new(db, events)
            .with_block_source(Some(Arc::new(ZeroBlockSource)));

        let block_size = 128 * 1024;
        let num_blocks = (*size).div_ceil(block_size);
        let mut blocks = Vec::with_capacity(num_blocks);
        for i in 0..num_blocks {
            let remaining = *size - (i * block_size);
            let this_block_size = remaining.min(block_size);
            blocks.push(BlockInfo {
                size: this_block_size as i32,
                hash: vec![0u8; 32],
                offset: (i * block_size) as i64,
            });
        }

        let file_info = FileInfo {
            name: "test.bin".to_string(),
            file_type: syncthing_core::types::FileType::File,
            size: *size as i64,
            permissions: 0o644,
            modified_s: 0,
            modified_ns: 0,
            version: syncthing_core::types::Vector::new(),
            sequence: 1,
            block_size: block_size as i32,
            blocks,
            symlink_target: None,
            deleted: Some(false),
        };

        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.to_async(&rt).iter(|| async {
                let stats = puller
                    .pull_folder(&folder,
                        vec![file_info.clone()],
                    )
                    .await
                    .expect("pull");
                black_box(stats);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_pull_single_file);
criterion_main!(benches);
