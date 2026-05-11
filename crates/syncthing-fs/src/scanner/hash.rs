//! Block hashing engine (TUNING_PLAN T-B1)
//!
//! CPU-bound SHA-256 hashing is offloaded to a dedicated rayon pool
//! via `tokio::task::spawn_blocking`.

use std::path::Path;
use std::sync::OnceLock;

use rayon::prelude::*;
use syncthing_core::{BlockHash, BlockInfo, Result, SyncthingError};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

/// Number of blocks to accumulate before dispatching to the parallel hash pool.
/// Batch size chosen to amortize spawn_blocking overhead while keeping memory
/// footprint reasonable (32 × 128 KiB ≈ 4 MiB per batch).
const HASH_BATCH_SIZE: usize = 32;

/// Thread pool dedicated to SHA-256 block hashing.
/// Isolated from tokio's blocking pool to avoid starving I/O tasks.
fn hash_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_cpus::get().max(2))
            .thread_name(|i| format!("hash-worker-{}", i))
            .build()
            .expect("hash thread pool")
    })
}

/// Compute block hashes for a file.
///
/// Reads the file in chunks and computes SHA-256 hashes for each block.
pub(super) async fn compute_block_hashes(path: &Path, block_size: usize) -> Result<Vec<BlockInfo>> {
    let mut file = File::open(path).await.map_err(SyncthingError::Io)?;
    let metadata = file.metadata().await.map_err(SyncthingError::Io)?;
    let file_size = metadata.len();

    let mut blocks = Vec::new();
    let mut offset: u64 = 0;

    loop {
        let mut batch: Vec<(u64, Vec<u8>)> = Vec::with_capacity(HASH_BATCH_SIZE);
        while batch.len() < HASH_BATCH_SIZE && offset < file_size {
            let mut chunk = vec![0u8; block_size];
            let n = file.read(&mut chunk).await.map_err(SyncthingError::Io)?;
            if n == 0 {
                break;
            }
            chunk.truncate(n);
            batch.push((offset, chunk));
            offset += n as u64;
        }
        if batch.is_empty() {
            break;
        }

        let hashed = tokio::task::spawn_blocking(move || {
            hash_pool().install(|| {
                batch
                    .into_par_iter()
                    .map(|(off, chunk)| BlockInfo {
                        hash: BlockHash::from_data(&chunk).to_vec(),
                        offset: off as i64,
                        size: chunk.len() as i32,
                    })
                    .collect::<Vec<BlockInfo>>()
            })
        })
        .await
        .map_err(|e| SyncthingError::io(format!("hash task failed: {}", e)))?;

        blocks.extend(hashed);
    }

    Ok(blocks)
}

/// Compute SHA-256 hash for a data block.
///
/// # Example
/// ```
/// use syncthing_fs::scanner::hash_block;
///
/// let data = b"hello world";
/// let hash = hash_block(data);
/// ```
pub fn hash_block(data: &[u8]) -> BlockHash {
    BlockHash::from_data(data)
}
