use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use kaidadb_common::{ChunkId, ChunkLocation, MediaManifest, Result, KaidaDbError};

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
}

struct IndexState {
    manifests: BTreeMap<String, MediaManifest>,
    chunk_locations: BTreeMap<String, ChunkLocation>, // keyed by hex chunk ID
    entry_count: u64,
    live_count: u64,
}

impl Index {
    pub fn open(data_dir: &Path) -> Result<Self> {
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

            for line in reader.lines() {
                let line = line?;
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<LogEntry>(&line) {
                    Ok(entry) => {
                        apply_entry(&mut state, entry);
                        state.entry_count += 1;
                    }
                    Err(e) => {
                        tracing::warn!(%e, "skipping corrupt log entry");
                    }
                }
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

        Ok(Self {
            state: RwLock::new(state),
            log_path,
            log_file: RwLock::new(log_file),
        })
    }

    fn append_entry(&self, entry: &LogEntry) -> Result<()> {
        let line = serde_json::to_string(entry)
            .map_err(|e| KaidaDbError::Serialization(e.to_string()))?;
        let mut file = self.log_file.write();
        writeln!(file, "{}", line)?;
        file.flush()?;
        Ok(())
    }

    // --- Media Manifest operations ---

    pub fn put_manifest(&self, manifest: &MediaManifest) -> Result<()> {
        let entry = LogEntry::PutManifest(manifest.clone());
        self.append_entry(&entry)?;
        self.state.write().manifests.insert(manifest.key.clone(), manifest.clone());
        Ok(())
    }

    pub fn get_manifest(&self, key: &str) -> Result<Option<MediaManifest>> {
        Ok(self.state.read().manifests.get(key).cloned())
    }

    pub fn delete_manifest(&self, key: &str) -> Result<()> {
        let entry = LogEntry::DeleteManifest(key.to_string());
        self.append_entry(&entry)?;
        self.state.write().manifests.remove(key);
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
        self.append_entry(&entry)?;
        self.state.write().chunk_locations.insert(hex, location.clone());
        Ok(())
    }

    pub fn get_chunk_location(&self, chunk_id: &ChunkId) -> Result<Option<ChunkLocation>> {
        Ok(self.state.read().chunk_locations.get(&chunk_id.to_hex()).cloned())
    }

    pub fn delete_chunk_location(&self, chunk_id: &ChunkId) -> Result<()> {
        let hex = chunk_id.to_hex();
        let entry = LogEntry::DeleteChunkLocation {
            chunk_id_hex: hex.clone(),
        };
        self.append_entry(&entry)?;
        self.state.write().chunk_locations.remove(&hex);
        Ok(())
    }

    /// Decrement ref count; delete location entry if it reaches zero. Returns true if deleted.
    pub fn decrement_chunk_ref(&self, chunk_id: &ChunkId) -> Result<bool> {
        let hex = chunk_id.to_hex();
        let state = self.state.write();

        if let Some(loc) = state.chunk_locations.get(&hex) {
            if loc.ref_count <= 1 {
                drop(state);
                self.delete_chunk_location(chunk_id)?;
                return Ok(true);
            }
            let mut updated = loc.clone();
            updated.ref_count -= 1;
            drop(state);
            self.put_chunk_location(chunk_id, &updated)?;
        }
        Ok(false)
    }

    /// Increment ref count for a chunk location.
    pub fn increment_chunk_ref(&self, chunk_id: &ChunkId) -> Result<()> {
        let hex = chunk_id.to_hex();
        let state = self.state.read();

        if let Some(loc) = state.chunk_locations.get(&hex) {
            let mut updated = loc.clone();
            updated.ref_count += 1;
            drop(state);
            self.put_chunk_location(chunk_id, &updated)?;
        }
        Ok(())
    }

    /// Compact the log by rewriting only live entries.
    pub fn compact(&self) -> Result<()> {
        let state = self.state.read();
        let tmp_path = self.log_path.with_extension("log.tmp");

        {
            let mut tmp = File::create(&tmp_path)?;
            for manifest in state.manifests.values() {
                let entry = LogEntry::PutManifest(manifest.clone());
                let line = serde_json::to_string(&entry)
                    .map_err(|e| KaidaDbError::Serialization(e.to_string()))?;
                writeln!(tmp, "{}", line)?;
            }
            for (hex, loc) in &state.chunk_locations {
                let entry = LogEntry::PutChunkLocation {
                    chunk_id_hex: hex.clone(),
                    location: loc.clone(),
                };
                let line = serde_json::to_string(&entry)
                    .map_err(|e| KaidaDbError::Serialization(e.to_string()))?;
                writeln!(tmp, "{}", line)?;
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

        tracing::info!("index log compacted");
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
}
