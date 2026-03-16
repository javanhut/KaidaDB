use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use kaidadb_common::{ChunkId, ChunkLocation, MediaManifest, Result, KaidaDbError};
use tokio::sync::mpsc;

use crate::blob_store::BlobStore;
use crate::index::Index;

/// High-level storage engine facade.
pub struct StorageEngine {
    data_dir: PathBuf,
    blob_store: BlobStore,
    index: Arc<Index>,
    chunk_size: usize,
}

impl StorageEngine {
    pub fn open(data_dir: &Path, chunk_size: usize) -> Result<Self> {
        let blob_store = BlobStore::new(data_dir)?;
        let index = Arc::new(Index::open(data_dir)?);

        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            blob_store,
            index,
            chunk_size,
        })
    }

    /// Store media from a contiguous byte buffer.
    pub fn store(&self, key: &str, data: &[u8], content_type: &str) -> Result<MediaManifest> {
        self.store_with_metadata(key, data, content_type, Default::default())
    }

    /// Store media with custom metadata.
    pub fn store_with_metadata(
        &self,
        key: &str,
        data: &[u8],
        content_type: &str,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<MediaManifest> {
        if key.is_empty() {
            return Err(KaidaDbError::InvalidKey("key cannot be empty".into()));
        }

        // Compute overall SHA-256 checksum
        let mut hasher = Sha256::new();
        hasher.update(data);
        let checksum = hex::encode(hasher.finalize());

        // Chunk the data
        let mut chunks = Vec::new();
        for chunk_data in data.chunks(self.chunk_size) {
            let chunk_id = ChunkId::from_data(chunk_data);

            // Write chunk file
            let path = self.blob_store.write_chunk(&chunk_id, chunk_data)?;

            // Update chunk location index
            match self.index.get_chunk_location(&chunk_id)? {
                Some(_) => {
                    self.index.increment_chunk_ref(&chunk_id)?;
                }
                None => {
                    let location = ChunkLocation {
                        path: path.to_string_lossy().to_string(),
                        ref_count: 1,
                    };
                    self.index.put_chunk_location(&chunk_id, &location)?;
                }
            }

            chunks.push(chunk_id);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let manifest = MediaManifest {
            key: key.to_string(),
            chunks,
            total_size: data.len() as u64,
            chunk_size: self.chunk_size,
            content_type: content_type.to_string(),
            checksum,
            metadata,
            created_at: now,
            updated_at: now,
        };

        self.index.put_manifest(&manifest)?;

        tracing::info!(key, size = data.len(), chunks = manifest.chunk_count(), "stored media");
        Ok(manifest)
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

        Ok(Bytes::from(buf))
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

        // We need to read from blob_store which isn't Send. Use a blocking task approach.
        // For now, read chunks inline on a blocking task.
        let blob_store_path = self.blob_store_path().to_path_buf();

        tokio::spawn(async move {
            let blob_store = match BlobStore::new(&blob_store_path) {
                Ok(bs) => bs,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };

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

    /// List media keys with pagination.
    pub fn list(
        &self,
        prefix: &str,
        limit: usize,
        cursor: &str,
    ) -> Result<(Vec<MediaManifest>, Option<String>)> {
        self.index.list_manifests(prefix, limit, cursor)
    }

    fn blob_store_path(&self) -> &Path {
        &self.data_dir
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
}
