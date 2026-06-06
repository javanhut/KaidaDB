use parking_lot::{Mutex, RwLock};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use kaidadb_common::{ChunkId, ChunkLocation, Durability, MediaManifest, Result, KaidaDbError};

/// Don't auto-compact until the log has at least this many entries — avoids
/// churning on tiny logs.
const COMPACTION_FLOOR: u64 = 1024;
/// Auto-compact when total log entries exceed this multiple of live entries.
const COMPACTION_RATIO: u64 = 2;

/// Log entry types for the append-only WAL.
#[derive(serde::Serialize, serde::Deserialize)]
enum LogEntry {
    PutManifest(MediaManifest),
    DeleteManifest(String),
    PutChunkLocation { chunk_id_hex: String, location: ChunkLocation },
    DeleteChunkLocation { chunk_id_hex: String },
}

/// File-backed index using an append-only log with in-memory BTreeMap.
///
/// On startup, replays the log to rebuild state. Compaction rewrites
/// the log with only live entries.
pub struct Index {
    state: RwLock<IndexState>,
    log_path: PathBuf,
    log_file: RwLock<File>,
    durability: Durability,
    /// Serializes chunk ref-count read-modify-write sequences so concurrent
    /// add/decrement on the same chunk can't race into a lost update or a
    /// double-delete.
    chunk_ref_lock: Mutex<()>,
    /// Total entries appended to the current log (since open or last compaction);
    /// drives auto-compaction.
    log_entries: AtomicU64,
    /// Ensures only one compaction runs at a time.
    compact_lock: Mutex<()>,
}

struct IndexState {
    manifests: BTreeMap<String, MediaManifest>,
    chunk_locations: BTreeMap<String, ChunkLocation>, // keyed by hex chunk ID
    entry_count: u64,
    live_count: u64,
}

impl Index {
    pub fn open(data_dir: &Path) -> Result<Self> {
        Self::open_with(data_dir, Durability::default())
    }

    pub fn open_with(data_dir: &Path, durability: Durability) -> Result<Self> {
        let index_dir = data_dir.join("index");
        fs::create_dir_all(&index_dir)?;

        let log_path = index_dir.join("index.log");

        // Replay existing log to rebuild state
        let mut state = IndexState {
            manifests: BTreeMap::new(),
            chunk_locations: BTreeMap::new(),
            entry_count: 0,
            live_count: 0,
        };

        if log_path.exists() {
            let file = File::open(&log_path)?;
            let reader = BufReader::new(file);

            // Track the byte offset of the last fully-valid entry. A crash
            // mid-append can leave a torn final line; we stop at the first parse
            // error, treat it as end-of-log, and truncate the tail rather than
            // skipping it and replaying later entries onto inconsistent state.
            let mut good_len: u64 = 0;
            let mut truncated = false;
            for line in reader.lines() {
                let line = line?;
                let line_bytes = line.len() as u64 + 1; // +1 for the '\n'
                if line.is_empty() {
                    good_len += line_bytes;
                    continue;
                }
                match serde_json::from_str::<LogEntry>(&line) {
                    Ok(entry) => {
                        apply_entry(&mut state, entry);
                        state.entry_count += 1;
                        good_len += line_bytes;
                    }
                    Err(e) => {
                        tracing::error!(
                            %e,
                            offset = good_len,
                            "corrupt WAL entry — truncating log at last good offset"
                        );
                        truncated = true;
                        break;
                    }
                }
            }

            if truncated {
                // Drop the torn tail so future appends start from a clean state.
                let f = OpenOptions::new().write(true).open(&log_path)?;
                f.set_len(good_len)?;
                f.sync_all()?;
            }

            state.live_count = state.manifests.len() as u64 + state.chunk_locations.len() as u64;
            tracing::info!(
                manifests = state.manifests.len(),
                chunks = state.chunk_locations.len(),
                log_entries = state.entry_count,
                "index rebuilt from log"
            );
        }

        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;

        let log_entries = AtomicU64::new(state.entry_count);
        Ok(Self {
            state: RwLock::new(state),
            log_path,
            log_file: RwLock::new(log_file),
            durability,
            chunk_ref_lock: Mutex::new(()),
            log_entries,
            compact_lock: Mutex::new(()),
        })
    }

    /// Append a log entry. When `sync` is true and durability is enabled, fsync
    /// the log so this entry (and all preceding ones) are on stable storage.
    /// `sync` should be set at operation boundaries (manifest commit / delete),
    /// which makes the cheaper chunk-location entries durable for free.
    fn append_entry(&self, entry: &LogEntry, sync: bool) -> Result<()> {
        let line = serde_json::to_string(entry)
            .map_err(|e| KaidaDbError::Serialization(e.to_string()))?;
        let mut file = self.log_file.write();
        writeln!(file, "{}", line)?;
        file.flush()?;
        if sync && self.durability == Durability::Sync {
            file.sync_data()?;
        }
        drop(file);
        self.log_entries.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Compact the log automatically once it grows well beyond the live set.
    fn maybe_compact(&self) {
        let entries = self.log_entries.load(Ordering::Relaxed);
        if entries < COMPACTION_FLOOR {
            return;
        }
        let live = {
            let s = self.state.read();
            (s.manifests.len() + s.chunk_locations.len()) as u64
        };
        if entries < live.saturating_mul(COMPACTION_RATIO) {
            return;
        }
        // Skip if another thread is already compacting.
        if let Some(_g) = self.compact_lock.try_lock() {
            if let Err(e) = self.compact() {
                tracing::warn!(%e, "auto-compaction failed");
            }
        }
    }

    // --- Media Manifest operations ---

    pub fn put_manifest(&self, manifest: &MediaManifest) -> Result<()> {
        let entry = LogEntry::PutManifest(manifest.clone());
        // Operation boundary: fsync makes this manifest and all the chunk-location
        // entries written before it durable in one shot.
        self.append_entry(&entry, true)?;
        self.state.write().manifests.insert(manifest.key.clone(), manifest.clone());
        self.maybe_compact();
        Ok(())
    }

    pub fn get_manifest(&self, key: &str) -> Result<Option<MediaManifest>> {
        Ok(self.state.read().manifests.get(key).cloned())
    }

    pub fn delete_manifest(&self, key: &str) -> Result<()> {
        let entry = LogEntry::DeleteManifest(key.to_string());
        // Operation boundary for delete/rename: fsync to make the removal (and any
        // preceding chunk-location updates) durable.
        self.append_entry(&entry, true)?;
        self.state.write().manifests.remove(key);
        self.maybe_compact();
        Ok(())
    }

    pub fn list_manifests(
        &self,
        prefix: &str,
        limit: usize,
        cursor: &str,
    ) -> Result<(Vec<MediaManifest>, Option<String>)> {
        let state = self.state.read();
        let mut results = Vec::new();
        let mut next_cursor = None;

        // BTreeMap range scan from cursor (or prefix start)
        let start = if cursor.is_empty() {
            prefix.to_string()
        } else {
            cursor.to_string()
        };

        for (key, manifest) in state.manifests.range(start..) {
            // Stop if past the prefix
            if !prefix.is_empty() && !key.starts_with(prefix) {
                break;
            }

            // Skip the cursor key itself
            if !cursor.is_empty() && key == cursor {
                continue;
            }

            results.push(manifest.clone());

            if results.len() >= limit {
                next_cursor = Some(key.clone());
                break;
            }
        }

        Ok((results, next_cursor))
    }

    // --- Chunk Location operations ---

    pub fn put_chunk_location(&self, chunk_id: &ChunkId, location: &ChunkLocation) -> Result<()> {
        let hex = chunk_id.to_hex();
        let entry = LogEntry::PutChunkLocation {
            chunk_id_hex: hex.clone(),
            location: location.clone(),
        };
        // Not synced individually — made durable by the manifest fsync that ends
        // the store operation. If we crash before that, the object isn't visible.
        self.append_entry(&entry, false)?;
        self.state.write().chunk_locations.insert(hex, location.clone());
        Ok(())
    }

    pub fn get_chunk_location(&self, chunk_id: &ChunkId) -> Result<Option<ChunkLocation>> {
        Ok(self.state.read().chunk_locations.get(&chunk_id.to_hex()).cloned())
    }

    /// Number of distinct chunk locations currently tracked.
    pub fn chunk_count(&self) -> usize {
        self.state.read().chunk_locations.len()
    }

    /// Snapshot of all tracked chunk-id hex strings (used by orphan GC).
    pub fn chunk_id_hexes(&self) -> Vec<String> {
        self.state.read().chunk_locations.keys().cloned().collect()
    }

    pub fn delete_chunk_location(&self, chunk_id: &ChunkId) -> Result<()> {
        let hex = chunk_id.to_hex();
        let entry = LogEntry::DeleteChunkLocation {
            chunk_id_hex: hex.clone(),
        };
        // Made durable by the trailing delete_manifest fsync in `delete`.
        self.append_entry(&entry, false)?;
        self.state.write().chunk_locations.remove(&hex);
        Ok(())
    }

    /// Take a reference on a chunk: create it at ref_count 1 if absent, otherwise
    /// increment. Atomic with respect to other ref-count mutations.
    pub fn add_chunk_ref(&self, chunk_id: &ChunkId, path: &str) -> Result<()> {
        let _guard = self.chunk_ref_lock.lock();
        let hex = chunk_id.to_hex();
        let current = self.state.read().chunk_locations.get(&hex).cloned();
        let updated = match current {
            Some(mut loc) => {
                loc.ref_count += 1;
                loc
            }
            None => ChunkLocation {
                path: path.to_string(),
                ref_count: 1,
            },
        };
        self.put_chunk_location(chunk_id, &updated)
    }

    /// Decrement ref count; delete location entry if it reaches zero. Returns true
    /// if deleted. The whole read-modify-write is serialized so two concurrent
    /// decrements can't both observe ref_count<=1 and double-delete.
    pub fn decrement_chunk_ref(&self, chunk_id: &ChunkId) -> Result<bool> {
        let _guard = self.chunk_ref_lock.lock();
        let hex = chunk_id.to_hex();
        let current = self.state.read().chunk_locations.get(&hex).cloned();

        if let Some(loc) = current {
            if loc.ref_count <= 1 {
                self.delete_chunk_location(chunk_id)?;
                return Ok(true);
            }
            let mut updated = loc;
            updated.ref_count -= 1;
            self.put_chunk_location(chunk_id, &updated)?;
        }
        Ok(false)
    }

    /// Compact the log by rewriting only live entries.
    pub fn compact(&self) -> Result<()> {
        let state = self.state.read();
        let tmp_path = self.log_path.with_extension("log.tmp");

        let mut written: u64 = 0;
        {
            let mut tmp = File::create(&tmp_path)?;
            for manifest in state.manifests.values() {
                let entry = LogEntry::PutManifest(manifest.clone());
                let line = serde_json::to_string(&entry)
                    .map_err(|e| KaidaDbError::Serialization(e.to_string()))?;
                writeln!(tmp, "{}", line)?;
                written += 1;
            }
            for (hex, loc) in &state.chunk_locations {
                let entry = LogEntry::PutChunkLocation {
                    chunk_id_hex: hex.clone(),
                    location: loc.clone(),
                };
                let line = serde_json::to_string(&entry)
                    .map_err(|e| KaidaDbError::Serialization(e.to_string()))?;
                writeln!(tmp, "{}", line)?;
                written += 1;
            }
            tmp.flush()?;
            tmp.sync_all()?;
        }

        drop(state);

        // Swap files atomically
        fs::rename(&tmp_path, &self.log_path)?;

        // Reopen log file for appending
        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;
        *self.log_file.write() = new_file;
        // The log now contains exactly `written` live entries.
        self.log_entries.store(written, Ordering::Relaxed);

        tracing::info!(entries = written, "index log compacted");
        Ok(())
    }
}

fn apply_entry(state: &mut IndexState, entry: LogEntry) {
    match entry {
        LogEntry::PutManifest(m) => {
            state.manifests.insert(m.key.clone(), m);
        }
        LogEntry::DeleteManifest(key) => {
            state.manifests.remove(&key);
        }
        LogEntry::PutChunkLocation { chunk_id_hex, location } => {
            state.chunk_locations.insert(chunk_id_hex, location);
        }
        LogEntry::DeleteChunkLocation { chunk_id_hex } => {
            state.chunk_locations.remove(&chunk_id_hex);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_crud() {
        let tmp = tempfile::tempdir().unwrap();
        let index = Index::open(tmp.path()).unwrap();

        let manifest = MediaManifest {
            key: "test-video".into(),
            chunks: vec![ChunkId::from_data(b"chunk1")],
            total_size: 1024,
            chunk_size: 2 * 1024 * 1024,
            content_type: "video/mp4".into(),
            checksum: "abc123".into(),
            metadata: Default::default(),
            created_at: 1000,
            updated_at: 1000,
        };

        index.put_manifest(&manifest).unwrap();

        let loaded = index.get_manifest("test-video").unwrap().unwrap();
        assert_eq!(loaded.key, "test-video");
        assert_eq!(loaded.total_size, 1024);

        assert!(index.get_manifest("nonexistent").unwrap().is_none());

        index.delete_manifest("test-video").unwrap();
        assert!(index.get_manifest("test-video").unwrap().is_none());
    }

    #[test]
    fn test_chunk_location_ref_counting() {
        let tmp = tempfile::tempdir().unwrap();
        let index = Index::open(tmp.path()).unwrap();

        let chunk_id = ChunkId::from_data(b"data");
        let loc = ChunkLocation {
            path: "/some/path.kdc".into(),
            ref_count: 2,
        };

        index.put_chunk_location(&chunk_id, &loc).unwrap();
        let loaded = index.get_chunk_location(&chunk_id).unwrap().unwrap();
        assert_eq!(loaded.ref_count, 2);

        // Decrement: 2 -> 1, not deleted
        assert!(!index.decrement_chunk_ref(&chunk_id).unwrap());
        let loaded = index.get_chunk_location(&chunk_id).unwrap().unwrap();
        assert_eq!(loaded.ref_count, 1);

        // Decrement: 1 -> deleted
        assert!(index.decrement_chunk_ref(&chunk_id).unwrap());
        assert!(index.get_chunk_location(&chunk_id).unwrap().is_none());
    }

    #[test]
    fn test_list_manifests() {
        let tmp = tempfile::tempdir().unwrap();
        let index = Index::open(tmp.path()).unwrap();

        for i in 0..5 {
            let manifest = MediaManifest {
                key: format!("videos/clip-{i:02}"),
                chunks: vec![],
                total_size: 0,
                chunk_size: 2 * 1024 * 1024,
                content_type: "video/mp4".into(),
                checksum: String::new(),
                metadata: Default::default(),
                created_at: 0,
                updated_at: 0,
            };
            index.put_manifest(&manifest).unwrap();
        }

        let (results, cursor) = index.list_manifests("videos/", 3, "").unwrap();
        assert_eq!(results.len(), 3);
        assert!(cursor.is_some());

        let (results2, _) = index
            .list_manifests("videos/", 3, &cursor.unwrap())
            .unwrap();
        assert_eq!(results2.len(), 2);
    }

    #[test]
    fn test_persistence_across_reopen() {
        let tmp = tempfile::tempdir().unwrap();

        // Write some data
        {
            let index = Index::open(tmp.path()).unwrap();
            let manifest = MediaManifest {
                key: "persist-test".into(),
                chunks: vec![ChunkId::from_data(b"c1")],
                total_size: 500,
                chunk_size: 2 * 1024 * 1024,
                content_type: "text/plain".into(),
                checksum: "xyz".into(),
                metadata: Default::default(),
                created_at: 100,
                updated_at: 100,
            };
            index.put_manifest(&manifest).unwrap();

            let chunk_id = ChunkId::from_data(b"c1");
            let loc = ChunkLocation {
                path: "chunks/ab/cd/hash.kdc".into(),
                ref_count: 1,
            };
            index.put_chunk_location(&chunk_id, &loc).unwrap();
        }

        // Reopen and verify
        {
            let index = Index::open(tmp.path()).unwrap();
            let manifest = index.get_manifest("persist-test").unwrap().unwrap();
            assert_eq!(manifest.total_size, 500);

            let chunk_id = ChunkId::from_data(b"c1");
            let loc = index.get_chunk_location(&chunk_id).unwrap().unwrap();
            assert_eq!(loc.ref_count, 1);
        }
    }

    #[test]
    fn test_compaction() {
        let tmp = tempfile::tempdir().unwrap();
        let index = Index::open(tmp.path()).unwrap();

        // Create and delete several entries to bloat the log
        for i in 0..10 {
            let manifest = MediaManifest {
                key: format!("compact-{i}"),
                chunks: vec![],
                total_size: 0,
                chunk_size: 2 * 1024 * 1024,
                content_type: "text/plain".into(),
                checksum: String::new(),
                metadata: Default::default(),
                created_at: 0,
                updated_at: 0,
            };
            index.put_manifest(&manifest).unwrap();
        }
        for i in 0..5 {
            index.delete_manifest(&format!("compact-{i}")).unwrap();
        }

        // Compact
        index.compact().unwrap();

        // Verify remaining entries survive
        for i in 5..10 {
            assert!(index.get_manifest(&format!("compact-{i}")).unwrap().is_some());
        }
        for i in 0..5 {
            assert!(index.get_manifest(&format!("compact-{i}")).unwrap().is_none());
        }
    }

    #[test]
    fn test_auto_compaction_shrinks_log() {
        let tmp = tempfile::tempdir().unwrap();
        let index = Index::open(tmp.path()).unwrap();

        // Repeatedly rewrite the SAME key: live set stays at 1, but the log grows
        // one entry per put. This must trip auto-compaction past the floor.
        for i in 0..(COMPACTION_FLOOR + 50) {
            index
                .put_manifest(&MediaManifest {
                    key: "hot".into(),
                    chunks: vec![],
                    total_size: i,
                    chunk_size: 1024,
                    content_type: "text/plain".into(),
                    checksum: String::new(),
                    metadata: Default::default(),
                    created_at: 0,
                    updated_at: 0,
                })
                .unwrap();
        }

        // After auto-compaction the log should be far smaller than the number of
        // appends (only live entries remain).
        let entries = index.log_entries.load(Ordering::Relaxed);
        assert!(
            entries < COMPACTION_FLOOR,
            "expected compaction to shrink log, got {entries} entries"
        );

        // State is still correct after compaction.
        let m = index.get_manifest("hot").unwrap().unwrap();
        assert_eq!(m.total_size, COMPACTION_FLOOR + 49);

        // And it survives a reopen (compacted log replays cleanly).
        drop(index);
        let reopened = Index::open(tmp.path()).unwrap();
        assert!(reopened.get_manifest("hot").unwrap().is_some());
    }

    #[test]
    fn test_torn_tail_is_truncated_on_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("index").join("index.log");

        // Write two good manifests.
        {
            let index = Index::open(tmp.path()).unwrap();
            for key in ["good-1", "good-2"] {
                index
                    .put_manifest(&MediaManifest {
                        key: key.into(),
                        chunks: vec![],
                        total_size: 1,
                        chunk_size: 1024,
                        content_type: "text/plain".into(),
                        checksum: String::new(),
                        metadata: Default::default(),
                        created_at: 0,
                        updated_at: 0,
                    })
                    .unwrap();
            }
        }

        // Simulate a crash mid-append: a torn (invalid JSON) final line.
        {
            use std::io::Write as _;
            let mut f = OpenOptions::new().append(true).open(&log_path).unwrap();
            f.write_all(b"{\"PutManifest\":{\"key\":\"torn\",\"chu").unwrap(); // no newline, truncated
        }

        // Reopen: the torn tail must be dropped, good entries intact, and the log
        // must be writable again (truncated to the last good offset).
        {
            let index = Index::open(tmp.path()).unwrap();
            assert!(index.get_manifest("good-1").unwrap().is_some());
            assert!(index.get_manifest("good-2").unwrap().is_some());
            assert!(index.get_manifest("torn").unwrap().is_none());

            // A fresh append after recovery must round-trip on the next reopen.
            index
                .put_manifest(&MediaManifest {
                    key: "after-recovery".into(),
                    chunks: vec![],
                    total_size: 2,
                    chunk_size: 1024,
                    content_type: "text/plain".into(),
                    checksum: String::new(),
                    metadata: Default::default(),
                    created_at: 0,
                    updated_at: 0,
                })
                .unwrap();
        }
        {
            let index = Index::open(tmp.path()).unwrap();
            assert!(index.get_manifest("after-recovery").unwrap().is_some());
            assert!(index.get_manifest("good-2").unwrap().is_some());
        }
    }
}
