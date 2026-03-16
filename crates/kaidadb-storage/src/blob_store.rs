use bytes::Bytes;
use memmap2::Mmap;
use std::fs;
use std::path::{Path, PathBuf};
use kaidadb_common::{ChunkId, Result, KaidaDbError};

use crate::chunk_format;

/// Manages chunk files on disk with two-level hex fan-out.
pub struct BlobStore {
    data_dir: PathBuf,
    chunks_dir: PathBuf,
}

impl BlobStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let chunks_dir = data_dir.join("chunks");
        fs::create_dir_all(&chunks_dir)?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            chunks_dir,
        })
    }

    /// Write a chunk to disk. Returns the file path.
    pub fn write_chunk(&self, chunk_id: &ChunkId, data: &[u8]) -> Result<PathBuf> {
        let path = self.chunk_path(chunk_id);

        // Skip write if already exists (content-addressed dedup)
        if path.exists() {
            return Ok(path);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let encoded = chunk_format::encode_chunk(data);

        // Write atomically via temp file + rename
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, &encoded)?;
        fs::rename(&tmp_path, &path)?;

        tracing::debug!(?chunk_id, "wrote chunk to disk");
        Ok(path)
    }

    /// Read a chunk from disk, verifying integrity.
    pub fn read_chunk(&self, chunk_id: &ChunkId) -> Result<Bytes> {
        let path = self.chunk_path(chunk_id);
        if !path.exists() {
            return Err(KaidaDbError::NotFound(format!(
                "chunk not found: {chunk_id}"
            )));
        }

        let file = fs::File::open(&path)?;
        // Safety: we only read the file, and it won't be modified while mapped
        // (chunks are immutable, content-addressed).
        let mmap = unsafe { Mmap::map(&file)? };
        chunk_format::decode_chunk(&mmap)
    }

    /// Delete a chunk from disk.
    pub fn delete_chunk(&self, chunk_id: &ChunkId) -> Result<()> {
        let path = self.chunk_path(chunk_id);
        if path.exists() {
            fs::remove_file(&path)?;
            tracing::debug!(?chunk_id, "deleted chunk from disk");
        }
        Ok(())
    }

    /// Check if a chunk exists on disk.
    pub fn chunk_exists(&self, chunk_id: &ChunkId) -> bool {
        self.chunk_path(chunk_id).exists()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn chunk_path(&self, chunk_id: &ChunkId) -> PathBuf {
        let (a, b) = chunk_id.fan_out();
        self.chunks_dir
            .join(a)
            .join(b)
            .join(format!("{}.kdc", chunk_id.to_hex()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blob_store_write_read() {
        let tmp = tempfile::tempdir().unwrap();
        let store = BlobStore::new(tmp.path()).unwrap();

        let data = b"test chunk data for blob store";
        let chunk_id = ChunkId::from_data(data);

        store.write_chunk(&chunk_id, data).unwrap();
        assert!(store.chunk_exists(&chunk_id));

        let read_back = store.read_chunk(&chunk_id).unwrap();
        assert_eq!(&read_back[..], data);
    }

    #[test]
    fn test_blob_store_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = BlobStore::new(tmp.path()).unwrap();

        let data = b"delete me";
        let chunk_id = ChunkId::from_data(data);

        store.write_chunk(&chunk_id, data).unwrap();
        assert!(store.chunk_exists(&chunk_id));

        store.delete_chunk(&chunk_id).unwrap();
        assert!(!store.chunk_exists(&chunk_id));
    }

    #[test]
    fn test_blob_store_dedup() {
        let tmp = tempfile::tempdir().unwrap();
        let store = BlobStore::new(tmp.path()).unwrap();

        let data = b"duplicate data";
        let chunk_id = ChunkId::from_data(data);

        let path1 = store.write_chunk(&chunk_id, data).unwrap();
        let path2 = store.write_chunk(&chunk_id, data).unwrap();
        assert_eq!(path1, path2);
    }
}
