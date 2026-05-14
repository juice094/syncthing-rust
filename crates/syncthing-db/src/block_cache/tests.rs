use super::*;
use tempfile::TempDir;

fn create_test_block_store() -> (CachedBlockStore, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let store = SledStore::open(temp_dir.path()).unwrap();
    let block_store = CachedBlockStore::new(store, 1024 * 1024).unwrap(); // 1MB cache
    (block_store, temp_dir)
}

#[tokio::test]
async fn test_block_store_put_get() {
    let (store, _temp) = create_test_block_store();

    let data = b"hello world";
    let hash = BlockHash::from_data(data);

    // Store block
    store.put(hash, data).await.unwrap();

    // Retrieve block
    let retrieved = store.get(hash).await.unwrap();
    assert_eq!(retrieved, Some(data.to_vec()));
}

#[tokio::test]
async fn test_block_store_has() {
    let (store, _temp) = create_test_block_store();

    let data = b"test data";
    let hash = BlockHash::from_data(data);

    assert!(!store.has(hash).await.unwrap());

    store.put(hash, data).await.unwrap();

    assert!(store.has(hash).await.unwrap());
}

#[tokio::test]
async fn test_block_store_delete() {
    let (store, _temp) = create_test_block_store();

    let data = b"delete me";
    let hash = BlockHash::from_data(data);

    store.put(hash, data).await.unwrap();
    assert!(store.has(hash).await.unwrap());

    store.delete(hash).await.unwrap();
    assert!(!store.has(hash).await.unwrap());
    assert_eq!(store.get(hash).await.unwrap(), None);
}

#[tokio::test]
async fn test_block_store_invalid_hash() {
    let (store, _temp) = create_test_block_store();

    let data = b"test data";
    let wrong_hash = BlockHash::from_bytes([0u8; 32]);

    let result = store.put(wrong_hash, data).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_cache_functionality() {
    let (store, _temp) = create_test_block_store();

    let data = b"cached data";
    let hash = BlockHash::from_data(data);

    // Put data (also adds to cache)
    store.put(hash, data).await.unwrap();

    // First get should be cache hit (because put adds to cache)
    let _ = store.get(hash).await.unwrap();
    let stats = store.cache_stats().await;
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 0);

    // Get again (should also be cache hit)
    let _ = store.get(hash).await.unwrap();
    let stats = store.cache_stats().await;
    assert_eq!(stats.hits, 2);
    assert_eq!(stats.misses, 0);
    assert_eq!(stats.hit_rate(), 1.0);
}

#[tokio::test]
async fn test_cache_eviction() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = CachedBlockStore::new(sled_store, 100).unwrap(); // Very small cache

    // Store multiple blocks that exceed cache size
    for i in 0..10 {
        let data = vec![i as u8; 20]; // 20 bytes each
        let hash = BlockHash::from_data(&data);
        store.put(hash, &data).await.unwrap();
    }

    // Cache should have evicted some entries
    let cache_size = store.cache_size().await;
    assert!(cache_size <= 100);
}

#[tokio::test]
async fn test_cache_clear() {
    let (store, _temp) = create_test_block_store();

    let data = b"clear me";
    let hash = BlockHash::from_data(data);

    store.put(hash, data).await.unwrap();
    store.get(hash).await.unwrap(); // Populate cache

    store.clear_cache().await;

    // After clear, should be a cache miss
    let _ = store.get(hash).await.unwrap();
    let stats = store.cache_stats().await;
    // One miss from initial get, one miss after clear
    assert!(stats.misses >= 1);
}

#[tokio::test]
async fn test_prefetch() {
    let (store, _temp) = create_test_block_store();

    let mut hashes = Vec::new();
    for i in 0..5 {
        let data = vec![i as u8; 100];
        let hash = BlockHash::from_data(&data);
        hashes.push(hash);
        store.put(hash, &data).await.unwrap();
    }

    store.clear_cache().await;

    // Prefetch should load into cache
    store.prefetch(&hashes).await.unwrap();

    // All should be cache hits now
    for hash in hashes {
        let _ = store.get(hash).await.unwrap();
    }

    let stats = store.cache_stats().await;
    // Prefetch puts items in cache without counting as hits
    // Subsequent gets should be hits
    assert_eq!(stats.hits, 5);
}

#[tokio::test]
async fn test_folder_index_operations() {
    let (store, _temp) = create_test_block_store();

    let folder = FolderId::new("test-folder");

    // Initially empty
    let index = store.get_index(&folder).await.unwrap();
    assert!(index.is_empty());

    // Update index
    let files = vec![FileInfo::new("file1.txt"), FileInfo::new("file2.txt")];

    store.update_index(&folder, files.clone()).await.unwrap();

    let index = store.get_index(&folder).await.unwrap();
    assert_eq!(index.len(), 2);

    // Delta update
    let mut new_file = FileInfo::new("file3.txt");
    new_file.sequence = 1;
    store
        .update_index_delta(&folder, vec![new_file])
        .await
        .unwrap();

    let index = store.get_index(&folder).await.unwrap();
    assert_eq!(index.len(), 3);
}

#[tokio::test]
async fn test_folder_stats() {
    let (store, _temp) = create_test_block_store();

    let folder = FolderId::new("test-folder");

    let stats = store.folder_stats(&folder).await.unwrap();
    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.total_bytes, 0);

    // Add files and update index
    let mut file = FileInfo::new("test.txt");
    file.size = 1000;
    store.update_index(&folder, vec![file]).await.unwrap();

    let stats = store.folder_stats(&folder).await.unwrap();
    assert_eq!(stats.file_count, 1);
    assert_eq!(stats.total_bytes, 1000);
}

#[tokio::test]
async fn test_concurrent_access() {
    let (store, _temp) = create_test_block_store();
    let store = Arc::new(store);

    let mut handles = vec![];

    // Spawn multiple tasks that write blocks
    for i in 0..10 {
        let store = store.clone();
        let handle = tokio::spawn(async move {
            let data = vec![i as u8; 1000];
            let hash = BlockHash::from_data(&data);
            store.put(hash, &data).await.unwrap();
            hash
        });
        handles.push(handle);
    }

    let hashes: Vec<BlockHash> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Verify all blocks are accessible
    for hash in hashes {
        assert!(store.has(hash).await.unwrap());
    }
}
