use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;

/// Default chunk size: 2 MiB
pub const DEFAULT_CHUNK_SIZE: usize = 2 * 1024 * 1024;

/// Minimum chunk size: 1 MiB
pub const MIN_CHUNK_SIZE: usize = 1024 * 1024;

/// Maximum chunk size: 16 MiB
pub const MAX_CHUNK_SIZE: usize = 16 * 1024 * 1024;

/// Chunk file magic bytes: "KDBC"
pub const CHUNK_MAGIC: [u8; 4] = [0x4B, 0x44, 0x42, 0x43];

/// Current chunk format version
pub const CHUNK_FORMAT_VERSION: u8 = 1;

/// Chunk header size in bytes
pub const CHUNK_HEADER_SIZE: usize = 18;

/// Content-addressed chunk identifier (SHA-256 hash)
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId(pub [u8; 32]);

impl ChunkId {
    pub fn from_data(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        ChunkId(id)
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        Ok(ChunkId(id))
    }

    /// Returns the two-level hex fan-out directory components (e.g., "ab", "cd")
    pub fn fan_out(&self) -> (String, String) {
        (hex::encode(&self.0[0..1]), hex::encode(&self.0[1..2]))
    }
}

impl fmt::Debug for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChunkId({})", &self.to_hex()[..16])
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Location of a chunk on disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkLocation {
    pub path: String,
    pub ref_count: u32,
}

/// Media manifest — stored in the index for each user key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaManifest {
    pub key: String,
    pub chunks: Vec<ChunkId>,
    pub total_size: u64,
    pub chunk_size: usize,
    pub content_type: String,
    pub checksum: String,
    pub metadata: HashMap<String, String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MediaManifest {
    pub fn chunk_count(&self) -> u32 {
        self.chunks.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_id_from_data() {
        let data = b"hello world";
        let id = ChunkId::from_data(data);
        let hex_str = id.to_hex();
        assert_eq!(hex_str.len(), 64);

        let id2 = ChunkId::from_data(data);
        assert_eq!(id, id2);

        let id3 = ChunkId::from_data(b"different data");
        assert_ne!(id, id3);
    }

    #[test]
    fn test_chunk_id_hex_roundtrip() {
        let data = b"test data";
        let id = ChunkId::from_data(data);
        let hex_str = id.to_hex();
        let id2 = ChunkId::from_hex(&hex_str).unwrap();
        assert_eq!(id, id2);
    }

    #[test]
    fn test_chunk_id_fan_out() {
        let id = ChunkId::from_data(b"test");
        let (a, b) = id.fan_out();
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 2);
    }
}
