//! Internal LRU cache for block data.

use syncthing_core::BlockHash;

/// LRU cache for frequently accessed blocks (O(1) operations via `lru` crate).
#[derive(Debug)]
pub(super) struct LruCache {
    entries: lru::LruCache<BlockHash, Vec<u8>>,
    max_size: usize,
    pub(super) current_size: usize,
}

impl LruCache {
    pub(super) fn new(max_size: usize) -> Self {
        // T-F2: max_size.max(1) 保证 >= 1，NonZeroUsize::new 不会返回 None
        let cap = std::num::NonZeroUsize::new(max_size.max(1)).expect("max(1) guarantees non-zero");
        Self {
            entries: lru::LruCache::new(cap),
            max_size,
            current_size: 0,
        }
    }

    pub(super) fn peek(&self, hash: &BlockHash) -> Option<Vec<u8>> {
        self.entries.peek(hash).cloned()
    }

    pub(super) fn touch(&mut self, hash: &BlockHash) -> bool {
        self.entries.get(hash).is_some()
    }

    pub(super) fn put(&mut self, hash: BlockHash, data: Vec<u8>) -> usize {
        let data_size = data.len();

        if let Some(old_data) = self.entries.pop(&hash) {
            self.current_size -= old_data.len();
        }

        let mut evicted = 0usize;
        while self.current_size + data_size > self.max_size && !self.entries.is_empty() {
            if let Some((_, old_data)) = self.entries.pop_lru() {
                self.current_size -= old_data.len();
                evicted += 1;
            } else {
                break;
            }
        }

        if data_size <= self.max_size {
            self.current_size += data_size;
            self.entries.put(hash, data);
        }
        evicted
    }

    pub(super) fn contains(&self, hash: &BlockHash) -> bool {
        self.entries.contains(hash)
    }

    pub(super) fn remove(&mut self, hash: &BlockHash) {
        if let Some(data) = self.entries.pop(hash) {
            self.current_size -= data.len();
        }
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.current_size = 0;
    }
}
