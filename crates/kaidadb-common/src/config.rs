use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{Result, KaidaDbError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaidaDbConfig {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: String,

    #[serde(default = "default_rest_addr")]
    pub rest_addr: String,

    #[serde(default)]
    pub storage: StorageConfig,

    #[serde(default)]
    pub cache: CacheConfig,

    #[serde(default)]
    pub streaming: StreamingConfig,
}

/// How aggressively the index write-ahead log is flushed to stable storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Durability {
    /// Never fsync — fastest, but acked writes can vanish on power loss.
    None,
    /// fsync at each operation boundary (manifest commit / delete / rename),
    /// which also flushes the preceding chunk-location entries. Durable and
    /// cheap (one fsync per object, not per chunk).
    Sync,
}

impl Default for Durability {
    fn default() -> Self {
        Durability::Sync
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,

    /// Maximum size of a single stored object in bytes. `0` means unlimited.
    /// Uploads exceeding this are rejected before exhausting memory/disk.
    #[serde(default = "default_max_object_size")]
    pub max_object_size: u64,

    /// Write-ahead log durability mode.
    #[serde(default)]
    pub durability: Durability,

    /// How often (seconds) to run orphan/fsck garbage collection. `0` disables.
    #[serde(default = "default_gc_interval_secs")]
    pub gc_interval_secs: u64,

    /// Orphan chunks younger than this (seconds) are left alone during GC so an
    /// in-flight upload isn't reclaimed out from under itself.
    #[serde(default = "default_gc_grace_secs")]
    pub gc_grace_secs: u64,

    /// Recompute and verify the full-object checksum on every read.
    #[serde(default)]
    pub verify_checksum_on_read: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Maximum cache size in bytes
    #[serde(default = "default_cache_max_size")]
    pub max_size: usize,

    /// Number of chunks to prefetch ahead during streaming
    #[serde(default = "default_prefetch_window")]
    pub prefetch_window: usize,

    /// Whether to cache first N chunks on write
    #[serde(default)]
    pub warm_on_write: bool,
}

impl Default for KaidaDbConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            grpc_addr: default_grpc_addr(),
            rest_addr: default_rest_addr(),
            storage: StorageConfig::default(),
            cache: CacheConfig::default(),
            streaming: StreamingConfig::default(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            chunk_size: default_chunk_size(),
            max_object_size: default_max_object_size(),
            durability: Durability::default(),
            gc_interval_secs: default_gc_interval_secs(),
            gc_grace_secs: default_gc_grace_secs(),
            verify_checksum_on_read: false,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size: default_cache_max_size(),
            prefetch_window: default_prefetch_window(),
            warm_on_write: false,
        }
    }
}

impl KaidaDbConfig {
    pub fn load(config_path: Option<&str>) -> Result<Self> {
        let mut figment = Figment::new();

        if let Some(path) = config_path {
            figment = figment.merge(Toml::file(path));
        }

        figment = figment.merge(Env::prefixed("KAIDADB_").split("_"));

        figment
            .extract()
            .map_err(|e| KaidaDbError::Config(e.to_string()))
    }

    pub fn validate(&self) -> Result<()> {
        if self.storage.chunk_size < crate::types::MIN_CHUNK_SIZE
            || self.storage.chunk_size > crate::types::MAX_CHUNK_SIZE
        {
            return Err(KaidaDbError::Config(format!(
                "chunk_size must be between {} and {} bytes",
                crate::types::MIN_CHUNK_SIZE,
                crate::types::MAX_CHUNK_SIZE
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingConfig {
    /// Default target segment duration in seconds (used in playlist generation)
    #[serde(default = "default_target_duration")]
    pub target_duration: f64,

    /// Base URL prefix for segment URLs in playlists.
    /// If empty, uses relative paths.
    #[serde(default)]
    pub base_url: String,

    /// Key prefix for all streaming content
    #[serde(default = "default_stream_prefix")]
    pub stream_prefix: String,

    /// Whether to include EXT-X-ENDLIST (VOD mode)
    #[serde(default = "default_vod_mode")]
    pub vod_mode: bool,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            target_duration: default_target_duration(),
            base_url: String::new(),
            stream_prefix: default_stream_prefix(),
            vod_mode: default_vod_mode(),
        }
    }
}

fn default_target_duration() -> f64 {
    4.0
}

fn default_stream_prefix() -> String {
    "streams/".to_string()
}

fn default_vod_mode() -> bool {
    true
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("./data")
}

fn default_grpc_addr() -> String {
    "0.0.0.0:50051".to_string()
}

fn default_rest_addr() -> String {
    "0.0.0.0:8080".to_string()
}

fn default_chunk_size() -> usize {
    crate::types::DEFAULT_CHUNK_SIZE
}

fn default_max_object_size() -> u64 {
    50 * 1024 * 1024 * 1024 // 50 GiB
}

fn default_gc_interval_secs() -> u64 {
    3600 // hourly
}

fn default_gc_grace_secs() -> u64 {
    3600 // 1 hour
}

fn default_cache_max_size() -> usize {
    512 * 1024 * 1024 // 512 MiB
}

fn default_prefetch_window() -> usize {
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = KaidaDbConfig::default();
        assert_eq!(config.storage.chunk_size, 2 * 1024 * 1024);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_chunk_size() {
        let mut config = KaidaDbConfig::default();
        config.storage.chunk_size = 100; // too small
        assert!(config.validate().is_err());
    }
}
