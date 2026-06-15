use bep_protocol::messages::{Hello, Index, WireBlockInfo, WireFileInfo, WireVector};
use criterion::{criterion_group, criterion_main, Criterion};
use prost::Message;
use std::hint::black_box;

fn bench_hello_encode_decode(c: &mut Criterion) {
    let hello = Hello::new("rust-node", "syncthing", "v0.2.6");
    c.bench_function("hello_encode_decode", |b| {
        b.iter(|| {
            let encoded = hello.encode_to_vec();
            let _ = Hello::decode(black_box(encoded.as_slice())).unwrap();
        });
    });
}

fn bench_index_encode_decode(c: &mut Criterion) {
    let mut files = Vec::new();
    for i in 0..100 {
        files.push(WireFileInfo {
            name: format!("file_{}.txt", i),
            r#type: 0,
            size: 1024,
            permissions: 0o644,
            modified_s: 0,
            deleted: false,
            invalid: false,
            no_permissions: false,
            version: Some(WireVector { counters: vec![] }),
            sequence: i as i64,
            modified_ns: 0,
            modified_by: 0,
            block_size: 128 * 1024,
            platform: None,
            blocks: vec![WireBlockInfo {
                offset: 0,
                size: 1024,
                hash: vec![0u8; 32],
            }],
            symlink_target: vec![],
            blocks_hash: vec![],
            encrypted: vec![],
            previous_blocks_hash: vec![],
        });
    }
    let index = Index {
        folder: "test".to_string(),
        files,
        last_sequence: 100,
    };
    c.bench_function("index_100files_encode_decode", |b| {
        b.iter(|| {
            let mut buf = Vec::new();
            index.encode(black_box(&mut buf)).unwrap();
            let _ = Index::decode(black_box(&*buf)).unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_hello_encode_decode,
    bench_index_encode_decode
);
criterion_main!(benches);
