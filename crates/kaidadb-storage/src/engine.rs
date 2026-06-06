use bytes::Bytes;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use kaidadb_common::{ChunkId, Durability, MediaManifest, Result, KaidaDbError};
use tokio::sync::mpsc;

/// Outcome of an orphan/fsck garbage-collection pass.
#[derive(Debug, Default, Clone)]
pub struct GcReport {
    /// Orphaned chunk files (on disk, unreferenced) that were deleted.
    pub orphans_removed: usize,
    /// Orphan candidates left alone because they were modified within the grace
    /// period (possibly an in-flight upload).
    pub orphans_skipped: usize,
    /// Index entries with no backing file on disk (data loss — logged, not fixed).
    pub dangling: usize,
}

use crate::blob_store::BlobStore;
use crate::index::Index;

/// Maximum allowed key length in bytes.
const MAX_KEY_LEN: usize = 1024;

/// Reject empty, over-long, or control-character keys. Keys are index identifiers
/// (blob paths are content-addressed, so this isn't a traversal boundary) — this
/// just keeps the index sane and avoids unbounded growth.
fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(KaidaDbError::InvalidKey("key cannot be empty".into()));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(KaidaDbError::InvalidKey(format!(
            "key exceeds {MAX_KEY_LEN} bytes"
        )));
    }
    if key.chars().any(|c| c.is_control()) {
        return Err(KaidaDbError::InvalidKey(
            "key contains control characters".into(),
        ));
    }
    Ok(())
}

/// Current unix time in seconds, tolerant of clock skew (never panics).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// High-level storage engine facade.
pub struct StorageEngine {
    data_dir: PathBuf,
    blob_store: Arc<BlobStore>,
    index: Arc<Index>,
    chunk_size: usize,
    /// Maximum size of a single object in bytes; `0` means unlimited.
    max_object_size: u64,
    /// Recompute and verify the full-object checksum on `read`.
    verify_on_read: bool,
    /// Per-key locks serializing mutations (store-commit / delete / rename) on the
    /// same key so they can't interleave and corrupt ref counts or visibility.
    key_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl StorageEngine {
    pub fn open(data_dir: &Path, chunk_size: usize) -> Result<Self> {
        Self::open_with(data_dir, chunk_size, Durability::default())
    }

    pub fn open_with(data_dir: &Path, chunk_size: usize, durability: Durability) -> Result<Self> {
        let blob_store = Arc::new(BlobStore::new(data_dir)?);
        let index = Arc::new(Index::open_with(data_dir, durability)?);

        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            blob_store,
            index,
            chunk_size,
            max_object_size: 0,
            verify_on_read: false,
            key_locks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Set the maximum single-object size (bytes). `0` = unlimited.
    pub fn with_max_object_size(mut self, max_object_size: u64) -> Self {
        self.max_object_size = max_object_size;
        self
    }

    /// Enable full-object checksum verification on `read`.
    pub fn with_verify_on_read(mut self, verify: bool) -> Self {
        self.verify_on_read = verify;
        self
    }

    /// Obtain the per-key mutex, creating it on demand. Prunes unused entries
    /// when the map grows so it can't leak one mutex per key ever touched.
    fn key_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut map = self.key_locks.lock();
        if map.len() > 1024 {
            // Only entries the map alone holds (strong_count == 1) are idle.
            map.retain(|_, v| Arc::strong_count(v) > 1);
        }
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Store media from a contiguous byte buffer.
    pub fn store(&self, key: &str, data: &[u8], content_type: &str) -> Result<MediaManifest> {
        self.store_with_metadata(key, data, content_type, Default::default())
    }

    /// Store media with custom metadata from a contiguous buffer.
    /// Thin wrapper over the incremental [`StoreSession`] API.
    pub fn store_with_metadata(
        &self,
        key: &str,
        data: &[u8],
        content_type: &str,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<MediaManifest> {
        let mut session = self.begin_store(key, content_type, metadata)?;
        session.write(data)?;
        session.finish()
    }

    /// Begin an incremental store. Feed bytes via [`StoreSession::write`] (chunks
    /// flush to disk as they fill, so the whole object is never held in memory),
    /// then call [`StoreSession::finish`]. Enforces `max_object_size`.
    pub fn begin_store(
        &self,
        key: &str,
        content_type: &str,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<StoreSession> {
        validate_key(key)?;
        Ok(StoreSession {
            blob_store: self.blob_store.clone(),
            index: self.index.clone(),
            key_lock: self.key_lock(key),
            chunk_size: self.chunk_size,
            max_object_size: self.max_object_size,
            key: key.to_string(),
            content_type: content_type.to_string(),
            metadata,
            hasher: Sha256::new(),
            total_size: 0,
            chunks: Vec::new(),
            buf: Vec::with_capacity(self.chunk_size),
            referenced_chunks: Vec::new(),
        })
    }

    /// Read entire media into a contiguous buffer.
    pub fn read(&self, key: &str) -> Result<Bytes> {
        let manifest = self
            .index
            .get_manifest(key)?
            .ok_or_else(|| KaidaDbError::NotFound(key.to_string()))?;

        let mut buf = Vec::with_capacity(manifest.total_size as usize);
        for chunk_id in &manifest.chunks {
            let chunk_data = self.blob_store.read_chunk(chunk_id)?;
            buf.extend_from_slice(&chunk_data);
        }

        if self.verify_on_read && !manifest.checksum.is_empty() {
            let actual = hex::encode(Sha256::digest(&buf));
            if actual != manifest.checksum {
                return Err(KaidaDbError::ChunkIntegrity {
                    chunk_id: key.to_string(),
                    detail: format!(
                        "full-object checksum mismatch: stored={}, computed={}",
                        manifest.checksum, actual
                    ),
                });
            }
        }

        Ok(Bytes::from(buf))
    }

    /// Garbage-collect orphaned chunk files (present on disk but unreferenced by
    /// the index) and report dangling references (indexed but missing on disk).
    /// Orphans modified within `grace` are skipped to avoid racing in-flight
    /// uploads. Safe to run concurrently with normal traffic.
    pub fn gc(&self, grace: Duration) -> Result<GcReport> {
        let indexed: HashSet<String> = self.index.chunk_id_hexes().into_iter().collect();
        let mut report = GcReport::default();
        let now = SystemTime::now();

        for (hex, path) in self.blob_store.iter_chunk_files()? {
            if indexed.contains(&hex) {
                continue;
            }
            // Skip recently-written files — a concurrent store may have written
            // the chunk but not yet recorded its index entry.
            let recent = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|mt| now.duration_since(mt).ok())
                .map(|age| age < grace)
                .unwrap_or(false);
            if recent {
                report.orphans_skipped += 1;
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => report.orphans_removed += 1,
                Err(e) => tracing::warn!(%e, ?path, "gc: failed to remove orphan chunk"),
            }
        }

        for hex in &indexed {
            if let Ok(id) = ChunkId::from_hex(hex) {
                if !self.blob_store.chunk_exists(&id) {
                    report.dangling += 1;
                }
            }
        }

        if report.orphans_removed > 0 || report.dangling > 0 {
            tracing::info!(?report, "gc complete");
        }
        Ok(report)
    }

    /// Read a byte range of media.
    pub fn read_range(&self, key: &str, offset: u64, length: u64) -> Result<Bytes> {
        let manifest = self
            .index
            .get_manifest(key)?
            .ok_or_else(|| KaidaDbError::NotFound(key.to_string()))?;

        let end = if length == 0 {
            manifest.total_size
        } else {
            (offset + length).min(manifest.total_size)
        };

        if offset >= manifest.total_size {
            return Ok(Bytes::new());
        }

        let chunk_size = manifest.chunk_size as u64;
        let start_chunk_idx = (offset / chunk_size) as usize;
        let end_chunk_idx = ((end - 1) / chunk_size) as usize;

        let mut buf = Vec::with_capacity((end - offset) as usize);

        for idx in start_chunk_idx..=end_chunk_idx {
            if idx >= manifest.chunks.len() {
                break;
            }
            let chunk_data = self.blob_store.read_chunk(&manifest.chunks[idx])?;
            let chunk_start = idx as u64 * chunk_size;

            let slice_start = if idx == start_chunk_idx {
                (offset - chunk_start) as usize
            } else {
                0
            };

            let slice_end = if idx == end_chunk_idx {
                (end - chunk_start) as usize
            } else {
                chunk_data.len()
            };

            let slice_end = slice_end.min(chunk_data.len());
            buf.extend_from_slice(&chunk_data[slice_start..slice_end]);
        }

        Ok(Bytes::from(buf))
    }

    /// Stream media chunks through an mpsc channel.
    /// The receiver yields `Bytes` for each chunk (or sub-chunk for range reads).
    pub fn stream(
        &self,
        key: &str,
        offset: u64,
        length: u64,
    ) -> Result<mpsc::Receiver<Result<Bytes>>> {
        let manifest = self
            .index
            .get_manifest(key)?
            .ok_or_else(|| KaidaDbError::NotFound(key.to_string()))?;

        let end = if length == 0 {
            manifest.total_size
        } else {
            (offset + length).min(manifest.total_size)
        };

        let (tx, rx) = mpsc::channel(4); // backpressure at 4 chunks

        if offset >= manifest.total_size {
            return Ok(rx);
        }

        let chunk_size = manifest.chunk_size as u64;
        let start_chunk_idx = (offset / chunk_size) as usize;
        let end_chunk_idx = ((end - 1) / chunk_size) as usize;

        // Collect chunk IDs to stream
        let chunk_ids: Vec<ChunkId> = manifest.chunks
            [start_chunk_idx..=end_chunk_idx.min(manifest.chunks.len() - 1)]
            .to_vec();

        // Share the engine's blob store rather than reconstructing one per stream.
        let blob_store = self.blob_store.clone();

        tokio::spawn(async move {
            for (i, chunk_id) in chunk_ids.iter().enumerate() {
                let chunk_data = match blob_store.read_chunk(chunk_id) {
                    Ok(data) => data,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                };

                let actual_idx = start_chunk_idx + i;
                let chunk_start = actual_idx as u64 * chunk_size;

                let slice_start = if actual_idx == start_chunk_idx {
                    (offset - chunk_start) as usize
                } else {
                    0
                };

                let slice_end = if actual_idx == end_chunk_idx {
                    (end - chunk_start) as usize
                } else {
                    chunk_data.len()
                };

                let slice_end = slice_end.min(chunk_data.len());
                let slice = Bytes::copy_from_slice(&chunk_data[slice_start..slice_end]);

                if tx.send(Ok(slice)).await.is_err() {
                    return; // Receiver dropped
                }
            }
        });

        Ok(rx)
    }

    /// Read a single chunk by its ID.
    pub fn read_chunk(&self, chunk_id: &ChunkId) -> Result<Bytes> {
        self.blob_store.read_chunk(chunk_id)
    }

    /// Get media metadata.
    pub fn get_manifest(&self, key: &str) -> Result<Option<MediaManifest>> {
        self.index.get_manifest(key)
    }

    /// Delete media, cleaning up unreferenced chunks.
    pub fn delete(&self, key: &str) -> Result<bool> {
        let lock = self.key_lock(key);
        let _guard = lock.lock();

        let manifest = match self.index.get_manifest(key)? {
            Some(m) => m,
            None => return Ok(false),
        };

        // Decrement ref counts; delete chunks with zero refs
        for chunk_id in &manifest.chunks {
            let should_delete = self.index.decrement_chunk_ref(chunk_id)?;
            if should_delete {
                self.blob_store.delete_chunk(chunk_id)?;
            }
        }

        self.index.delete_manifest(key)?;
        tracing::info!(key, "deleted media");
        Ok(true)
    }

    /// Rename (move) media from one key to another.
    /// This only updates the manifest key — chunks are untouched.
    pub fn rename(&self, from_key: &str, to_key: &str) -> Result<MediaManifest> {
        validate_key(to_key)?;
        if from_key == to_key {
            return Err(KaidaDbError::InvalidKey("source and destination keys are the same".into()));
        }

        // Lock both keys in a consistent (sorted) order to avoid deadlock with a
        // concurrent rename in the opposite direction.
        let (a, b) = if from_key < to_key {
            (from_key, to_key)
        } else {
            (to_key, from_key)
        };
        let lock_a = self.key_lock(a);
        let lock_b = self.key_lock(b);
        let _ga = lock_a.lock();
        let _gb = lock_b.lock();

        let manifest = self
            .index
            .get_manifest(from_key)?
            .ok_or_else(|| KaidaDbError::NotFound(from_key.to_string()))?;

        if self.index.get_manifest(to_key)?.is_some() {
            return Err(KaidaDbError::AlreadyExists(to_key.to_string()));
        }

        let new_manifest = MediaManifest {
            key: to_key.to_string(),
            updated_at: now_secs(),
            ..manifest
        };

        self.index.put_manifest(&new_manifest)?;
        self.index.delete_manifest(from_key)?;

        tracing::info!(from = from_key, to = to_key, "renamed media");
        Ok(new_manifest)
    }

    /// List media keys with pagination.
    pub fn list(
        &self,
        prefix: &str,
        limit: usize,
        cursor: &str,
    ) -> Result<(Vec<MediaManifest>, Option<String>)> {
        self.index.list_manifests(prefix, limit, cursor)
    }

    /// Path to the engine's data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Number of distinct chunks currently tracked by the index.
    pub fn chunk_location_count(&self) -> usize {
        self.index.chunk_count()
    }
}

/// An in-progress streaming store. Bytes fed via [`StoreSession::write`] are
/// accumulated into the current chunk and flushed to the blob store as each
/// chunk fills, so the full object is never buffered in memory. Call
/// [`StoreSession::finish`] to commit the manifest, or drop/[`StoreSession::abort`]
/// to discard (no manifest is written, so a partial upload never becomes visible).
pub struct StoreSession {
    blob_store: Arc<BlobStore>,
    index: Arc<Index>,
    key_lock: Arc<Mutex<()>>,
    chunk_size: usize,
    max_object_size: u64,
    key: String,
    content_type: String,
    metadata: std::collections::HashMap<String, String>,
    hasher: Sha256,
    total_size: u64,
    chunks: Vec<ChunkId>,
    buf: Vec<u8>,
    /// Every chunk this session took a reference on (created or incremented),
    /// in order. Used by rollback so a failed upload leaves no leaked refs or
    /// orphaned chunk files.
    referenced_chunks: Vec<ChunkId>,
}

impl StoreSession {
    /// Number of bytes accepted so far (including the un-flushed tail).
    pub fn bytes_written(&self) -> u64 {
        self.total_size + self.buf.len() as u64
    }

    /// Feed bytes into the session, flushing full chunks to disk as they fill.
    pub fn write(&mut self, mut data: &[u8]) -> Result<()> {
        if self.max_object_size != 0
            && self.bytes_written() + data.len() as u64 > self.max_object_size
        {
            return Err(KaidaDbError::TooLarge(format!(
                "object exceeds max_object_size of {} bytes",
                self.max_object_size
            )));
        }
        while !data.is_empty() {
            let space = self.chunk_size - self.buf.len();
            let take = space.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() >= self.chunk_size {
                self.flush_chunk()?;
            }
        }
        Ok(())
    }

    /// Flush the current buffer as a content-addressed chunk.
    fn flush_chunk(&mut self) -> Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk_data = std::mem::replace(&mut self.buf, Vec::with_capacity(self.chunk_size));
        let chunk_id = ChunkId::from_data(&chunk_data);

        let path = self.blob_store.write_chunk(&chunk_id, &chunk_data)?;
        // Atomic get-or-create-or-increment so concurrent stores of the same new
        // chunk can't both create it at ref_count 1 (which would undercount).
        self.index
            .add_chunk_ref(&chunk_id, &path.to_string_lossy())?;
        self.referenced_chunks.push(chunk_id.clone());

        self.hasher.update(&chunk_data);
        self.total_size += chunk_data.len() as u64;
        self.chunks.push(chunk_id);
        Ok(())
    }

    /// Commit the upload: flush the trailing chunk and write the manifest.
    pub fn finish(mut self) -> Result<MediaManifest> {
        self.flush_chunk()?;

        let checksum = hex::encode(std::mem::take(&mut self.hasher).finalize());
        let now = now_secs();
        let manifest = MediaManifest {
            key: std::mem::take(&mut self.key),
            chunks: std::mem::take(&mut self.chunks),
            total_size: self.total_size,
            chunk_size: self.chunk_size,
            content_type: std::mem::take(&mut self.content_type),
            checksum,
            metadata: std::mem::take(&mut self.metadata),
            created_at: now,
            updated_at: now,
        };

        // Serialize the commit against concurrent delete/rename/store on this key.
        let key_lock = self.key_lock.clone();
        let _guard = key_lock.lock();

        // If we're overwriting an existing object, release the previous version's
        // chunk references after committing the new manifest, so overwrites don't
        // leak the old chunks. (This session already holds refs on the new
        // chunks, so any shared chunks net out correctly.)
        let previous = self.index.get_manifest(&manifest.key)?;
        self.index.put_manifest(&manifest)?;
        if let Some(old) = previous {
            for chunk_id in &old.chunks {
                if self.index.decrement_chunk_ref(chunk_id)? {
                    let _ = self.blob_store.delete_chunk(chunk_id);
                }
            }
        }

        tracing::info!(
            key = %manifest.key,
            size = manifest.total_size,
            chunks = manifest.chunk_count(),
            "stored media"
        );
        Ok(manifest)
    }

    /// Abort the upload, releasing every reference this session took so a failed
    /// upload leaves no leaked refs or orphaned chunk files. (Dropping without
    /// calling this simply leaves the chunks for the orphan GC to reclaim later.)
    pub fn abort(mut self) {
        for chunk_id in std::mem::take(&mut self.referenced_chunks) {
            // Mirror `delete`: drop one ref and remove the file if it hit zero.
            match self.index.decrement_chunk_ref(&chunk_id) {
                Ok(true) => {
                    let _ = self.blob_store.delete_chunk(&chunk_id);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, StorageEngine) {
        let tmp = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(tmp.path(), 1024).unwrap(); // 1KB chunks for testing
        (tmp, engine)
    }

    #[test]
    fn test_store_and_read() {
        let (_tmp, engine) = setup();

        let data = vec![42u8; 5000]; // ~5 chunks at 1KB
        let manifest = engine.store("test-key", &data, "application/octet-stream").unwrap();

        assert_eq!(manifest.total_size, 5000);
        assert_eq!(manifest.chunk_count(), 5);

        let read_back = engine.read("test-key").unwrap();
        assert_eq!(&read_back[..], &data[..]);
    }

    #[test]
    fn test_read_range() {
        let (_tmp, engine) = setup();

        let data: Vec<u8> = (0..5000u16).map(|i| (i % 256) as u8).collect();
        engine.store("test-key", &data, "application/octet-stream").unwrap();

        // Read from middle of first chunk
        let range = engine.read_range("test-key", 100, 200).unwrap();
        assert_eq!(&range[..], &data[100..300]);

        // Read spanning chunks
        let range = engine.read_range("test-key", 900, 300).unwrap();
        assert_eq!(&range[..], &data[900..1200]);

        // Read to end
        let range = engine.read_range("test-key", 4800, 0).unwrap();
        assert_eq!(&range[..], &data[4800..]);
    }

    #[test]
    fn test_delete() {
        let (_tmp, engine) = setup();

        let data = vec![1u8; 2000];
        engine.store("del-key", &data, "video/mp4").unwrap();

        assert!(engine.delete("del-key").unwrap());
        assert!(engine.read("del-key").is_err());
        assert!(!engine.delete("del-key").unwrap());
    }

    #[test]
    fn test_not_found() {
        let (_tmp, engine) = setup();
        assert!(engine.read("nope").is_err());
    }

    #[test]
    fn test_list() {
        let (_tmp, engine) = setup();

        engine.store("a/1", b"data1", "text/plain").unwrap();
        engine.store("a/2", b"data2", "text/plain").unwrap();
        engine.store("b/1", b"data3", "text/plain").unwrap();

        let (results, _) = engine.list("a/", 10, "").unwrap();
        assert_eq!(results.len(), 2);

        let (results, _) = engine.list("", 10, "").unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_stream() {
        let (_tmp, engine) = setup();

        let data: Vec<u8> = (0..3000u16).map(|i| (i % 256) as u8).collect();
        engine.store("stream-key", &data, "video/mp4").unwrap();

        let mut rx = engine.stream("stream-key", 0, 0).unwrap();
        let mut collected = Vec::new();
        while let Some(chunk) = rx.recv().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(collected, data);
    }

    #[tokio::test]
    async fn test_stream_range() {
        let (_tmp, engine) = setup();

        let data: Vec<u8> = (0..3000u16).map(|i| (i % 256) as u8).collect();
        engine.store("stream-key", &data, "video/mp4").unwrap();

        let mut rx = engine.stream("stream-key", 500, 1500).unwrap();
        let mut collected = Vec::new();
        while let Some(chunk) = rx.recv().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(collected, &data[500..2000]);
    }

    #[test]
    fn test_store_session_matches_buffered() {
        // A multi-chunk incremental session must produce a manifest identical to
        // the old whole-buffer store (same chunks, same checksum).
        let (_tmp, engine) = setup();
        let data: Vec<u8> = (0..5000u16).map(|i| (i % 256) as u8).collect();

        let buffered = engine.store("buffered", &data, "video/mp4").unwrap();

        // Feed the session in deliberately awkward, sub-chunk-sized writes.
        let mut session = engine
            .begin_store("session", "video/mp4", Default::default())
            .unwrap();
        for piece in data.chunks(333) {
            session.write(piece).unwrap();
        }
        let streamed = session.finish().unwrap();

        assert_eq!(streamed.total_size, buffered.total_size);
        assert_eq!(streamed.checksum, buffered.checksum);
        assert_eq!(streamed.chunks, buffered.chunks);
        assert_eq!(&engine.read("session").unwrap()[..], &data[..]);
    }

    #[test]
    fn test_store_session_size_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(tmp.path(), 1024)
            .unwrap()
            .with_max_object_size(2048);

        let mut session = engine
            .begin_store("too-big", "application/octet-stream", Default::default())
            .unwrap();
        session.write(&vec![0u8; 1500]).unwrap();
        // Crossing the 2048-byte cap must fail.
        let err = session.write(&vec![0u8; 1000]).unwrap_err();
        assert!(matches!(err, KaidaDbError::TooLarge(_)));
        session.abort();

        // Nothing should have been committed.
        assert!(engine.read("too-big").is_err());
    }

    #[test]
    fn test_overwrite_reclaims_old_chunks() {
        let (_tmp, engine) = setup();

        // Two distinct single-chunk objects under the same key.
        engine.store("k", &vec![1u8; 512], "text/plain").unwrap();
        assert_eq!(engine.chunk_location_count(), 1);
        engine.store("k", &vec![2u8; 512], "text/plain").unwrap();

        // Overwrite must not leak the old chunk — still exactly one tracked.
        assert_eq!(engine.chunk_location_count(), 1);
        assert_eq!(&engine.read("k").unwrap()[..], &vec![2u8; 512][..]);
    }

    #[test]
    fn test_concurrent_store_delete_shared_chunk() {
        use std::sync::Arc;
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let engine = Arc::new(StorageEngine::open(tmp.path(), 1024).unwrap());

        // All objects share identical content → one deduplicated chunk.
        let payload = vec![42u8; 800];

        let mut handles = Vec::new();
        for t in 0..8 {
            let engine = engine.clone();
            let payload = payload.clone();
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    let key = format!("k-{t}-{i}");
                    engine.store(&key, &payload, "application/octet-stream").unwrap();
                    engine.delete(&key).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // Every object created was deleted; the shared chunk must be fully
        // reclaimed (no ref-count underflow leaving it alive, no double-free).
        assert_eq!(engine.chunk_location_count(), 0);

        // A fresh store of the same content must still work (chunk re-created).
        engine.store("final", &payload, "application/octet-stream").unwrap();
        assert_eq!(&engine.read("final").unwrap()[..], &payload[..]);
        assert_eq!(engine.chunk_location_count(), 1);
    }

    #[test]
    fn test_key_validation() {
        let (_tmp, engine) = setup();

        // Empty, over-long, and control-character keys are rejected.
        assert!(engine.store("", b"x", "text/plain").is_err());
        let long = "a".repeat(2000);
        assert!(engine.store(&long, b"x", "text/plain").is_err());
        assert!(engine.store("bad\nkey", b"x", "text/plain").is_err());

        // A normal nested key with slashes is fine.
        assert!(engine.store("tv/show/s01/e01.mp4", b"x", "video/mp4").is_ok());
    }

    #[test]
    fn test_gc_removes_orphans_and_keeps_live() {
        let (tmp, engine) = setup();

        // One live object.
        engine.store("live", &vec![3u8; 600], "text/plain").unwrap();

        // Plant an orphan chunk file directly under chunks/ (no index entry),
        // back-dated so it's outside the grace window.
        let orphan_id = ChunkId::from_data(b"orphan-content");
        let (a, b) = orphan_id.fan_out();
        let dir = tmp.path().join("chunks").join(&a).join(&b);
        std::fs::create_dir_all(&dir).unwrap();
        let orphan_path = dir.join(format!("{}.kdc", orphan_id.to_hex()));
        std::fs::write(&orphan_path, b"junk").unwrap();

        let report = engine.gc(Duration::from_secs(0)).unwrap();
        assert_eq!(report.orphans_removed, 1);
        assert_eq!(report.dangling, 0);
        assert!(!orphan_path.exists());

        // Live data untouched.
        assert_eq!(&engine.read("live").unwrap()[..], &vec![3u8; 600][..]);
    }

    #[test]
    fn test_gc_grace_period_skips_recent_orphans() {
        let (tmp, engine) = setup();
        let orphan_id = ChunkId::from_data(b"fresh-orphan");
        let (a, b) = orphan_id.fan_out();
        let dir = tmp.path().join("chunks").join(&a).join(&b);
        std::fs::create_dir_all(&dir).unwrap();
        let orphan_path = dir.join(format!("{}.kdc", orphan_id.to_hex()));
        std::fs::write(&orphan_path, b"junk").unwrap();

        // Large grace window → the just-written orphan is left alone.
        let report = engine.gc(Duration::from_secs(3600)).unwrap();
        assert_eq!(report.orphans_removed, 0);
        assert_eq!(report.orphans_skipped, 1);
        assert!(orphan_path.exists());
    }

    #[test]
    fn test_verify_on_read_detects_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(tmp.path(), 1024)
            .unwrap()
            .with_verify_on_read(true);

        let data = vec![5u8; 1500];
        let manifest = engine.store("v", &data, "text/plain").unwrap();
        assert_eq!(&engine.read("v").unwrap()[..], &data[..]); // clean read ok

        // Corrupt the first chunk file on disk.
        let (a, b) = manifest.chunks[0].fan_out();
        let path = tmp
            .path()
            .join("chunks")
            .join(&a)
            .join(&b)
            .join(format!("{}.kdc", manifest.chunks[0].to_hex()));
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();

        // The per-chunk CRC catches this first; either way, the read must fail.
        assert!(engine.read("v").is_err());
    }

    #[test]
    fn test_store_session_abort_rolls_back_chunks() {
        let (_tmp, engine) = setup();

        // Pre-existing object sharing a chunk we'll also reference, to prove abort
        // only releases this session's refs and never deletes a still-live chunk.
        let shared = vec![7u8; 1024];
        engine.store("keeper", &shared, "text/plain").unwrap();

        let mut session = engine
            .begin_store("doomed", "text/plain", Default::default())
            .unwrap();
        session.write(&shared).unwrap(); // dedup: increments the shared chunk
        session.write(&vec![9u8; 1024]).unwrap(); // unique chunk: created 0->1
        session.abort();

        // The shared chunk survives (keeper still readable); the unique chunk is gone.
        assert_eq!(&engine.read("keeper").unwrap()[..], &shared[..]);
        assert!(engine.read("doomed").is_err());
    }
}
