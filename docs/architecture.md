# KaidaDB Architecture

## The Big Picture

KaidaDB is a specialized database for media files. Unlike general-purpose databases that store rows and columns, KaidaDB stores large binary files (videos, audio, images) and is optimized for streaming them back efficiently.

Here's the simple version of how it works:

1. **You send a file** to KaidaDB (say, a 500 MB video)
2. **KaidaDB chops it into chunks** (250 pieces of 2 MB each)
3. **Each chunk gets a fingerprint** (SHA-256 hash) — if that exact chunk already exists, it's not stored again
4. **A manifest** records which chunks belong to your file and in what order
5. **When you play it back**, KaidaDB reads the chunks in order and streams them to you, with a cache keeping popular chunks in memory for speed

No external databases. No cloud services. Just files on disk, organized intelligently.

## Workspace Structure

KaidaDB is written in Rust and organized as a workspace of 8 crates (libraries and binaries). Each crate has one job:

```
KaidaDB/
├── crates/
│   ├── kaidadb-common/     Shared types, errors, and configuration
│   ├── kaidadb-storage/    Storage engine (chunks, index, disk I/O)
│   ├── kaidadb-cache/      In-memory LRU chunk cache
│   ├── kaidadb-api/        gRPC + REST API layer + streaming playlists
│   ├── kaidadb-server/     Server binary (starts everything)
│   ├── kaidadb-cli/        Command-line client
│   ├── kaidadb-tui/        Terminal user interface
│   └── kaidadb-cluster/    Distributed clustering (future)
├── proto/kaidadb.proto     gRPC service definition
├── config.toml             Default configuration
├── install.sh              Installer script
└── service/                Service files (systemd, OpenRC, kaidadb-ctl)
```

### How the Crates Depend on Each Other

```
kaidadb-server ─┬─► kaidadb-api ─┬─► kaidadb-storage ─► kaidadb-common
kaidadb-cli ────┘                └─► kaidadb-cache ────► kaidadb-common
kaidadb-tui ────────────────────────────────────────────► kaidadb-common
```

- **common** is the foundation — types, errors, and config that everything shares
- **storage** handles all disk operations (reading, writing, indexing chunks)
- **cache** provides a fast in-memory layer on top of storage
- **api** wraps storage + cache with gRPC and REST interfaces
- **server** is the binary that wires everything together and starts listening
- **cli** and **tui** are client binaries that talk to the server over gRPC

## Storage Engine

The storage engine (`kaidadb-storage`) is the core of KaidaDB. It manages three things: the **blob store** (chunk files on disk), the **index** (which chunks belong to which media), and the **write/read paths** that tie them together.

### On-Disk Layout

```
data/
├── index/
│   └── index.log           Append-only write-ahead log (JSON lines)
└── chunks/
    └── a1/
        └── b2/
            └── a1b2c3d4...kdc    Chunk file
```

The `chunks/` directory uses a **two-level fan-out** based on the first two bytes of each chunk's SHA-256 hash. This prevents any single directory from accumulating too many files, which would slow down filesystem operations. With 256 x 256 possible subdirectories, the chunks are evenly distributed.

### Chunk File Format (.kdc)

Each chunk is stored in a `.kdc` file with a small header:

```
Byte 0-3:   Magic bytes (0x4B444243 = "KDBC")
Byte 4:     Format version (currently 1)
Byte 5:     Flags (reserved)
Byte 6-9:   CRC32 checksum of the payload (little-endian)
Byte 10-17: Payload length (little-endian u64)
Byte 18+:   Payload data (the actual chunk bytes)
```

Every time a chunk is read, KaidaDB verifies the magic bytes, version, and CRC32 checksum. If any check fails, the chunk is considered corrupt and an error is returned. This means data integrity is verified on every read — not just at write time.

### The Write Path

When you store a file, here's what happens step by step:

1. **Receive the data** — via gRPC streaming or REST PUT body
2. **Compute the overall checksum** — SHA-256 of the entire file
3. **Split into chunks** — fixed-size pieces (default 2 MiB, configurable 1-16 MiB)
4. **For each chunk:**
   - Compute its SHA-256 hash → this becomes the `ChunkId`
   - Check if a chunk file with that hash already exists on disk
   - If it exists: increment the reference count (deduplication — no data written)
   - If it's new: write it to disk as a `.kdc` file using atomic rename (write to temp file first, then rename into place — prevents partial writes)
5. **Create a MediaManifest** — records the key, ordered list of chunk IDs, total size, content type, checksum, custom metadata, and timestamps
6. **Append the manifest to the index log** — a single JSON line written to `index.log`

### The Read Path

When you retrieve a file (or a byte range):

1. **Look up the MediaManifest** by key in the in-memory BTreeMap — O(1) lookup
2. **Calculate which chunks are needed** — from the byte offset and length requested, compute the start and end chunk indices. This is O(1) because chunks are fixed-size: `chunk_index = byte_offset / chunk_size`
3. **For each chunk:**
   - Check the LRU cache — if it's there, use it (no disk I/O)
   - On cache miss: memory-map the `.kdc` file from disk
   - Verify CRC32 integrity
   - Slice to the exact byte range needed (partial chunks at the start/end of a range)
   - Insert into the cache for future reads
4. **Stream to the client** — chunks are sent through a bounded async channel (capacity of 4-16) so the client controls the pace. If the client reads slowly, KaidaDB pauses rather than buffering everything in memory. This is called **backpressure**.

### The Delete Path

1. Look up the manifest
2. For each chunk in the manifest, decrement its reference count
3. If a chunk's reference count reaches zero (no other media uses it), delete the `.kdc` file from disk
4. Remove the manifest from the index

## Index

The index is KaidaDB's "table of contents." It tracks two things:

- **Which media exists** — a mapping from key → MediaManifest
- **Where chunks live on disk** — a mapping from ChunkId → file path + reference count

### How It Works

The index uses a **hybrid approach**:

- **On disk:** An append-only log file (`index.log`) where every operation is recorded as a JSON line
- **In memory:** Two sorted maps (BTreeMaps) for fast lookups

When KaidaDB starts, it replays the log file from top to bottom to rebuild the in-memory maps. This is fast — even millions of entries replay in seconds because it's sequential I/O.

### Log Entry Types

```
PutManifest       — A new media object was stored (or overwritten)
DeleteManifest    — A media object was deleted
PutChunkLocation  — A new chunk was written to disk
DeleteChunkLocation — A chunk was removed from disk
```

### Compaction

Over time, the log file grows with entries for deleted media. **Compaction** rewrites the log with only the live entries, then atomically swaps the old file for the new one. This is safe — if the process crashes during compaction, the old log is still intact.

### Why Not a "Real" Database?

The append-only log + BTreeMap approach is intentionally simple:

- No dependency on SQLite, RocksDB, or any external engine
- The log is human-readable (JSON lines) — you can inspect it with any text editor
- BTreeMap gives sorted key iteration for free, which powers prefix-based listing
- The entire index lives in memory, so lookups are nanoseconds, not milliseconds

The tradeoff is that the index must fit in RAM. For media storage, this is rarely a problem — even 1 million media objects with their manifests use only a few hundred megabytes of RAM, while the media files themselves might be terabytes on disk.

## Deduplication

KaidaDB uses **content-addressed storage** — each chunk is identified by its SHA-256 hash, not by which file it belongs to. If two different files happen to contain an identical 2 MiB block of data, only one copy is stored on disk.

### How It Saves Space

Real-world examples where deduplication helps:

- **TV shows** with the same intro sequence — the intro chunks are stored once regardless of how many episodes share them
- **Music remixes** that sample the same source track
- **Updated versions** of a file where most content is unchanged — only the changed chunks are written
- **Multiple resolutions** that share audio tracks

### Reference Counting

Each chunk tracks how many manifests reference it. When you store media, referenced chunks get their count incremented. When you delete media, counts are decremented. A chunk is only deleted from disk when its count reaches zero.

This means you can safely delete one copy of a file even if another file shares some of its chunks — the shared chunks remain on disk until all references are gone.

## Cache

KaidaDB includes a **size-bounded LRU (Least Recently Used) cache** for chunks. The cache sits between the API layer and the storage engine.

### How It Works

- The cache stores recently-accessed chunks in memory, keyed by ChunkId
- Maximum size is configurable (default: 512 MiB)
- When the cache is full and a new chunk needs to be inserted, the least-recently-used chunk is evicted
- Chunks larger than the total cache size are never cached (they'd evict everything)

### Prefetch Window

During sequential streaming (playing a video from start to finish), KaidaDB **prefetches** the next few chunks into the cache before they're requested. The prefetch window is configurable (default: 3 chunks ahead). This eliminates disk I/O latency for the common case of sequential playback.

### Cache Warming on Write

Optionally, when you store media, the first N chunks can be immediately placed in the cache (`warm_on_write` config option). This is useful when you expect media to be played back shortly after upload.

### Cache Statistics

The cache tracks hit/miss counts:

```
CacheStats {
    hits: 15234,          // Chunks served from memory
    misses: 892,          // Chunks that required disk I/O
    current_size: 412 MB, // Current memory usage
    max_size: 512 MB,     // Configured limit
    entry_count: 206,     // Number of chunks in cache
}
```

A high hit rate means most reads are served from memory with zero disk I/O.

## API Layer

KaidaDB exposes two APIs that share the same underlying storage engine and cache:

### gRPC (Port 50051)

Used by the CLI and TUI. Offers streaming uploads and downloads with backpressure. Defined in `proto/kaidadb.proto`. Best for:

- Programmatic access from other Rust, Go, Python, etc. services
- Large file uploads (client-streaming)
- Low-latency operations

### REST (Port 8080)

Standard HTTP API. Best for:

- Web browsers and web applications
- `curl` and scripting
- Media players (VLC, hls.js, etc.) — they understand HTTP Range requests natively
- CDN integration (CDNs cache HTTP responses)

Both APIs are served concurrently by the same server process using `tokio::select!`. They share the same `StorageEngine` and `ChunkCache` instances (wrapped in `Arc` for thread-safe shared ownership).

### Streaming Playlists

The API layer also handles **HLS and DASH playlist generation** for adaptive bitrate streaming. This is a thin layer that queries the storage engine for segments and assembles playlist files on-the-fly. See the [Streaming Guide](./streaming-guide.md) for details.

## MediaManifest

Every stored media object has a manifest that records everything about it:

```rust
MediaManifest {
    key: "tv/breaking-bad/s01/e01",    // The lookup key
    chunks: [ChunkId(...), ...],        // Ordered list of chunk hashes
    total_size: 1_234_567_890,          // Total bytes
    chunk_size: 2_097_152,              // Bytes per chunk (2 MiB)
    content_type: "video/mp4",          // MIME type
    checksum: "a1b2c3d4...",            // SHA-256 of entire file
    metadata: {"resolution": "4k"},     // Custom key-value pairs
    created_at: 1710000000,             // Unix timestamp
    updated_at: 1710000000,             // Unix timestamp
}
```

The chunk list is ordered — chunk 0 is the start of the file, the last chunk is the end. This ordering, combined with fixed chunk sizes, is what makes byte-range lookups O(1).

## Design Decisions

### Why Fixed-Size Chunks?

Variable-size chunks (content-defined chunking) give better deduplication for files that shift content around. But fixed-size chunks give:

- **O(1) byte-to-chunk mapping** — `chunk_index = byte_offset / chunk_size`, no need to scan a chunk table
- **Simpler implementation** — no rolling hash, no minimum/maximum chunk size tuning
- **Predictable I/O** — every chunk read is the same size (good for cache management)

For media files, where seeking is common and content rarely shifts, fixed-size chunks are the better tradeoff.

### Why Memory-Mapped I/O?

Chunk files are read using `mmap` (memory-mapped file I/O). The operating system handles paging — only the requested bytes are actually loaded from disk, and the OS can cache pages across reads. This is more efficient than `read()` for random access patterns and avoids copying data between kernel and userspace buffers.

### Why BTreeMap Instead of HashMap?

BTreeMap keeps keys sorted, which makes prefix-based listing ("show me everything under `tv/breaking-bad/`") a fast range scan. HashMap would require scanning all keys and filtering.

## Roadmap

- **Phase 1** (current): Single-node storage engine with gRPC + REST APIs, HLS/DASH streaming
- **Phase 2**: Cache benchmarks, prefetch optimizations, metrics dashboard
- **Phase 3**: Distributed cluster — Raft consensus, consistent hashing, replication
- **Phase 4**: Failure detection, anti-entropy repair, TLS
- **Phase 5**: Operational tooling, hot-reload, backup/restore
