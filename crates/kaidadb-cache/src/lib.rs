use bytes::Bytes;
use lru::LruCache;
use parking_lot::RwLock;
use std::num::NonZeroUsize;
use kaidadb_common::ChunkId;

/// Size-bounded LRU cache for chunk data.
pub struct ChunkCache {
    inner: RwLock<CacheInner>,
    max_size: usize,
}

struct CacheInner {
    cache: LruCache<ChunkId, Bytes>,
    current_size: usize,
    hits: u64,
    misses: u64,
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub current_size: usize,
    pub max_size: usize,
    pub entry_count: usize,
}

impl ChunkCache {
    pub fn new(max_size: usize) -> Self {
        // Estimate max entries based on typical chunk size (2MiB)
        let estimated_entries = (max_size / (2 * 1024 * 1024)).max(64);
        Self {
            inner: RwLock::new(CacheInner {
                cache: LruCache::new(NonZeroUsize::new(estimated_entries).unwrap()),
                current_size: 0,
                hits: 0,
                misses: 0,
            }),
            max_size,
        }
    }

    /// Get a chunk from cache. Returns None on cache miss.
    pub fn get(&self, chunk_id: &ChunkId) -> Option<Bytes> {
        let mut inner = self.inner.write();
        match inner.cache.get(chunk_id) {
            Some(data) => {
                let cloned = data.clone();
                inner.hits += 1;
                Some(cloned)
            }
            None => {
                inner.misses += 1;
                None
            }
        }
    }

    /// Insert a chunk into the cache, evicting LRU entries if necessary.
    pub fn insert(&self, chunk_id: ChunkId, data: Bytes) {
        let data_len = data.len();
        let mut inner = self.inner.write();

        // Don't cache if single chunk exceeds max size
        if data_len > self.max_size {
            return;
        }

        // Evict until we have space
        while inner.current_size + data_len > self.max_size {
            if let Some((_evicted_id, evicted_data)) = inner.cache.pop_lru() {
                inner.current_size -= evicted_data.len();
            } else {
                break;
            }
        }

        // If we're replacing an existing entry, subtract its size
        if let Some(old) = inner.cache.put(chunk_id, data) {
            inner.current_size -= old.len();
        }
        inner.current_size += data_len;
    }

    /// Remove a chunk from the cache.
    pub fn invalidate(&self, chunk_id: &ChunkId) {
        let mut inner = self.inner.write();
        if let Some(data) = inner.cache.pop(chunk_id) {
            inner.current_size -= data.len();
        }
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.read();
        CacheStats {
            hits: inner.hits,
            misses: inner.misses,
            current_size: inner.current_size,
            max_size: self.max_size,
            entry_count: inner.cache.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_hit_miss() {
        let cache = ChunkCache::new(1024 * 1024); // 1MB
        let id = ChunkId::from_data(b"test");
        let data = Bytes::from_static(b"hello");

        assert!(cache.get(&id).is_none());
        assert_eq!(cache.stats().misses, 1);

        cache.insert(id.clone(), data.clone());

        let result = cache.get(&id);
        assert!(result.is_some());
        assert_eq!(&result.unwrap()[..], b"hello");
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn test_cache_eviction() {
        let cache = ChunkCache::new(100); // Tiny cache

        let id1 = ChunkId::from_data(b"chunk1");
        let id2 = ChunkId::from_data(b"chunk2");

        cache.insert(id1.clone(), Bytes::from(vec![0u8; 60]));
        cache.insert(id2.clone(), Bytes::from(vec![1u8; 60]));

        // id1 should have been evicted to make room for id2
        assert!(cache.get(&id1).is_none());
        assert!(cache.get(&id2).is_some());
    }

    #[test]
    fn test_cache_invalidate() {
        let cache = ChunkCache::new(1024 * 1024);
        let id = ChunkId::from_data(b"data");

        cache.insert(id.clone(), Bytes::from_static(b"value"));
        assert!(cache.get(&id).is_some());

        cache.invalidate(&id);
        assert!(cache.get(&id).is_none());
        assert_eq!(cache.stats().current_size, 0);
    }

    #[test]
    fn test_cache_size_tracking() {
        let cache = ChunkCache::new(1024 * 1024);

        let id1 = ChunkId::from_data(b"a");
        let id2 = ChunkId::from_data(b"b");

        cache.insert(id1, Bytes::from(vec![0u8; 100]));
        cache.insert(id2, Bytes::from(vec![0u8; 200]));

        assert_eq!(cache.stats().current_size, 300);
        assert_eq!(cache.stats().entry_count, 2);
    }
}
