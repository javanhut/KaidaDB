use kaidadb_cache::ChunkCache;
use kaidadb_common::KaidaDbConfig;
use kaidadb_storage::StorageEngine;
use tempfile::tempdir;

fn setup() -> (tempfile::TempDir, StorageEngine) {
    let tmp = tempdir().unwrap();
    let engine = StorageEngine::open(tmp.path(), 1024).unwrap(); // 1KB chunks for tests
    (tmp, engine)
}

#[test]
fn test_store_read_roundtrip() {
    let (_tmp, engine) = setup();

    let data = vec![0xABu8; 5000];
    let manifest = engine
        .store("test/video.mp4", &data, "video/mp4")
        .unwrap();

    assert_eq!(manifest.total_size, 5000);
    assert_eq!(manifest.content_type, "video/mp4");
    assert_eq!(manifest.chunk_count(), 5);

    let read_back = engine.read("test/video.mp4").unwrap();
    assert_eq!(&read_back[..], &data[..]);
}

#[test]
fn test_range_read() {
    let (_tmp, engine) = setup();

    let data: Vec<u8> = (0..4096u32).map(|i| (i % 256) as u8).collect();
    engine
        .store("range-test", &data, "application/octet-stream")
        .unwrap();

    // Range within a single chunk
    let range = engine.read_range("range-test", 0, 512).unwrap();
    assert_eq!(&range[..], &data[0..512]);

    // Range spanning chunk boundary
    let range = engine.read_range("range-test", 900, 300).unwrap();
    assert_eq!(&range[..], &data[900..1200]);

    // Range to end
    let range = engine.read_range("range-test", 3500, 0).unwrap();
    assert_eq!(&range[..], &data[3500..]);

    // Range past end returns what's available
    let range = engine.read_range("range-test", 4000, 500).unwrap();
    assert_eq!(&range[..], &data[4000..]);
}

#[test]
fn test_delete_cleanup() {
    let (_tmp, engine) = setup();

    let data = vec![42u8; 2000];
    engine.store("del-test", &data, "video/mp4").unwrap();

    assert!(engine.get_manifest("del-test").unwrap().is_some());

    assert!(engine.delete("del-test").unwrap());

    assert!(engine.get_manifest("del-test").unwrap().is_none());
    assert!(engine.read("del-test").is_err());
}

#[test]
fn test_overwrite() {
    let (_tmp, engine) = setup();

    engine
        .store("overwrite-key", b"first version", "text/plain")
        .unwrap();
    engine
        .store("overwrite-key", b"second version", "text/plain")
        .unwrap();

    let data = engine.read("overwrite-key").unwrap();
    assert_eq!(&data[..], b"second version");
}

#[test]
fn test_list_with_prefix() {
    let (_tmp, engine) = setup();

    engine.store("videos/a.mp4", b"a", "video/mp4").unwrap();
    engine.store("videos/b.mp4", b"b", "video/mp4").unwrap();
    engine.store("audio/c.mp3", b"c", "audio/mpeg").unwrap();

    let (videos, _) = engine.list("videos/", 100, "").unwrap();
    assert_eq!(videos.len(), 2);

    let (audio, _) = engine.list("audio/", 100, "").unwrap();
    assert_eq!(audio.len(), 1);

    let (all, _) = engine.list("", 100, "").unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn test_list_pagination() {
    let (_tmp, engine) = setup();

    for i in 0..10 {
        engine
            .store(&format!("item-{i:02}"), b"data", "text/plain")
            .unwrap();
    }

    let (page1, cursor1) = engine.list("", 3, "").unwrap();
    assert_eq!(page1.len(), 3);
    assert!(cursor1.is_some());

    let (page2, cursor2) = engine.list("", 3, &cursor1.unwrap()).unwrap();
    assert_eq!(page2.len(), 3);
    assert!(cursor2.is_some());
}

#[test]
fn test_dedup_shared_chunks() {
    let (_tmp, engine) = setup();

    let data = vec![0xFFu8; 2000];
    engine
        .store("copy-a", &data, "application/octet-stream")
        .unwrap();
    engine
        .store("copy-b", &data, "application/octet-stream")
        .unwrap();

    let a = engine.read("copy-a").unwrap();
    let b = engine.read("copy-b").unwrap();
    assert_eq!(&a[..], &data[..]);
    assert_eq!(&b[..], &data[..]);

    // Delete one — the other should still work since chunks are ref-counted
    engine.delete("copy-a").unwrap();
    let b = engine.read("copy-b").unwrap();
    assert_eq!(&b[..], &data[..]);
}

#[tokio::test]
async fn test_stream_roundtrip() {
    let (_tmp, engine) = setup();

    let data: Vec<u8> = (0..5000u16).map(|i| (i % 256) as u8).collect();
    engine.store("stream-test", &data, "video/mp4").unwrap();

    let mut rx = engine.stream("stream-test", 0, 0).unwrap();
    let mut collected = Vec::new();
    while let Some(chunk) = rx.recv().await {
        collected.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(collected, data);
}

#[tokio::test]
async fn test_stream_range() {
    let (_tmp, engine) = setup();

    let data: Vec<u8> = (0..5000u16).map(|i| (i % 256) as u8).collect();
    engine.store("stream-range", &data, "video/mp4").unwrap();

    let mut rx = engine.stream("stream-range", 1000, 2000).unwrap();
    let mut collected = Vec::new();
    while let Some(chunk) = rx.recv().await {
        collected.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(collected, &data[1000..3000]);
}

#[test]
fn test_cache_integration() {
    let (_tmp, engine) = setup();
    let cache = ChunkCache::new(1024 * 1024);

    let data = vec![42u8; 2000];
    let manifest = engine.store("cached-key", &data, "video/mp4").unwrap();

    // Manually populate cache
    for chunk_id in &manifest.chunks {
        let chunk_data = engine.read_chunk(chunk_id).unwrap();
        cache.insert(chunk_id.clone(), chunk_data);
    }

    // Verify cache hits
    for chunk_id in &manifest.chunks {
        assert!(cache.get(chunk_id).is_some());
    }

    let stats = cache.stats();
    assert_eq!(stats.hits, manifest.chunk_count() as u64);
    assert!(stats.current_size > 0);
}

#[test]
fn test_empty_key_rejected() {
    let (_tmp, engine) = setup();
    assert!(engine.store("", b"data", "text/plain").is_err());
}

#[test]
fn test_not_found() {
    let (_tmp, engine) = setup();
    assert!(engine.read("nonexistent").is_err());
    assert!(!engine.delete("nonexistent").unwrap());
}

#[test]
fn test_config_defaults() {
    let config = KaidaDbConfig::default();
    assert!(config.validate().is_ok());
    assert_eq!(config.storage.chunk_size, 2 * 1024 * 1024);
    assert_eq!(config.grpc_addr, "0.0.0.0:50051");
    assert_eq!(config.rest_addr, "0.0.0.0:8080");
}

#[test]
fn test_large_media_roundtrip() {
    let (_tmp, engine) = setup();

    // 50KB file with known pattern, chunked at 1KB = 50 chunks
    let data: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
    let manifest = engine.store("large-file", &data, "video/mp4").unwrap();

    assert_eq!(manifest.total_size, 50_000);
    assert_eq!(manifest.chunk_count(), 49); // ceil(50000/1024) = 49

    let read_back = engine.read("large-file").unwrap();
    assert_eq!(&read_back[..], &data[..]);
}

#[test]
fn test_persistence_roundtrip() {
    let tmp = tempdir().unwrap();

    let data = vec![99u8; 3000];

    // Store data
    {
        let engine = StorageEngine::open(tmp.path(), 1024).unwrap();
        engine.store("persist-key", &data, "video/mp4").unwrap();
    }

    // Reopen and read
    {
        let engine = StorageEngine::open(tmp.path(), 1024).unwrap();
        let read_back = engine.read("persist-key").unwrap();
        assert_eq!(&read_back[..], &data[..]);
    }
}
