# KaidaDB

A Rust database purpose-built for storing and streaming media. KaidaDB provides fast key-value media access with content-addressed chunk deduplication, LRU caching, and dual gRPC/REST APIs.

## Features

- **Content-addressed storage** — chunks are SHA-256 hashed; identical data is stored once
- **Streaming delivery** — memory-mapped I/O with backpressure-aware async channels
- **Range requests** — seek to any byte offset with O(1) chunk lookup
- **LRU caching** — size-bounded chunk cache for hot data
- **Dual API** — gRPC for performance, REST with `Range` header support for compatibility
- **Durability** — append-only write-ahead log with in-memory BTreeMap index, rebuilt on startup
- **Zero external database dependencies** — pure Rust storage engine

## Quick Start

### Build

```bash
cargo build --release
```

### Run the Server

```bash
# With defaults (gRPC :50051, REST :8080, data in ./data)
cargo run -p kaidadb-server

# With a config file
cargo run -p kaidadb-server -- --config config.toml
```

### Store and Retrieve Media

**CLI:**

```bash
# Store a file
kaidadb-cli store my-video ./sample.mp4

# Retrieve it
kaidadb-cli get my-video -o output.mp4

# Check metadata
kaidadb-cli meta my-video

# List all media
kaidadb-cli list

# Delete
kaidadb-cli delete my-video
```

**REST:**

```bash
# Upload
curl -X PUT -H "Content-Type: video/mp4" -T sample.mp4 http://localhost:8080/v1/media/my-video

# Download
curl http://localhost:8080/v1/media/my-video -o output.mp4

# Range request (first 1 MiB)
curl -H "Range: bytes=0-1048575" http://localhost:8080/v1/media/my-video -o partial.bin

# Metadata
curl http://localhost:8080/v1/meta/my-video

# List
curl http://localhost:8080/v1/media?prefix=videos/&limit=50

# Delete
curl -X DELETE http://localhost:8080/v1/media/my-video

# Health check
curl http://localhost:8080/v1/health
```

## Architecture

```
KaidaDB/
├── Cargo.toml                    # Workspace root
├── proto/kaidadb.proto          # gRPC service definition
├── config.toml                   # Default configuration
├── crates/
│   ├── kaidadb-common/          # Shared types, errors, config
│   ├── kaidadb-storage/         # Storage engine (chunks, index, mmap)
│   ├── kaidadb-cache/           # LRU caching layer
│   ├── kaidadb-cluster/         # Distributed consensus (Phase 3)
│   ├── kaidadb-api/             # gRPC + REST gateway
│   ├── kaidadb-server/          # Server binary
│   └── kaidadb-cli/             # CLI client binary
```

**Dependency flow:** `server/cli` → `api` → `storage + cache` → `common`

## Configuration

KaidaDB loads configuration from three sources (in priority order):

1. Environment variables (prefix `KAIDADB_`)
2. TOML config file (passed via `--config`)
3. Built-in defaults

### Config File

```toml
data_dir = "./data"
grpc_addr = "0.0.0.0:50051"
rest_addr = "0.0.0.0:8080"

[storage]
chunk_size = 2097152       # 2 MiB (valid range: 1-16 MiB)

[cache]
max_size = 536870912       # 512 MiB
prefetch_window = 3
warm_on_write = false
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `KAIDADB_DATA_DIR` | `./data` | Storage directory |
| `KAIDADB_GRPC_ADDR` | `0.0.0.0:50051` | gRPC listen address |
| `KAIDADB_REST_ADDR` | `0.0.0.0:8080` | REST listen address |
| `KAIDADB_STORAGE_CHUNK_SIZE` | `2097152` | Chunk size in bytes (1-16 MiB) |
| `KAIDADB_CACHE_MAX_SIZE` | `536870912` | Max cache size in bytes |
| `KAIDADB_CACHE_PREFETCH_WINDOW` | `3` | Chunks to prefetch during streaming |
| `KAIDADB_CACHE_WARM_ON_WRITE` | `false` | Cache first chunks on write |

## Storage Engine

### On-Disk Layout

```
data/
├── index/
│   └── index.log       # Append-only write-ahead log (JSON lines)
└── chunks/
    └── ab/cd/           # Two-level hex fan-out (first two bytes of hash)
        └── <sha256>.kdc # Chunk file
```

### Write Path

1. Client sends data (via gRPC stream or REST PUT body)
2. Data is split into fixed-size chunks (default 2 MiB)
3. Each chunk is SHA-256 hashed to produce a `ChunkId`
4. Chunk file is written to disk in `.kdc` format (skipped if already exists — dedup)
5. Chunk location is recorded in the index with a reference count
6. A `MediaManifest` is created with the ordered chunk list and stored in the index
7. Overall SHA-256 checksum is computed and stored in the manifest

### Read Path

1. Look up `MediaManifest` by key
2. Compute which chunks cover the requested byte range
3. For each chunk: check LRU cache → cache miss: mmap the `.kdc` file from disk
4. Verify CRC32 integrity on read
5. Slice the chunk to the exact byte range requested
6. Stream `Bytes` to client via bounded mpsc channel (backpressure at 4 chunks)

### Chunk File Format (.kdc)

```
Offset  Size   Field
──────  ─────  ─────────────────────────
0       4      Magic bytes: 0x4B444243 ("KDBC")
4       1      Format version (currently 1)
5       1      Flags (reserved, currently 0)
6       4      CRC32 of payload (little-endian)
10      8      Payload length (little-endian u64)
18      N      Payload bytes
```

Every read verifies the magic bytes, format version, and CRC32 checksum.

### Index

The index uses an **append-only JSON lines log** with an **in-memory BTreeMap**.

On startup, the log file is replayed line-by-line to rebuild two BTreeMaps:
- **Manifests:** `key → MediaManifest` (media metadata + chunk list)
- **Chunk locations:** `chunk_id_hex → ChunkLocation` (file path + reference count)

Log entry types:
```
PutManifest, DeleteManifest, PutChunkLocation, DeleteChunkLocation
```

**Compaction** rewrites the log with only live entries, then atomically swaps the file.

### Deduplication

Chunks are content-addressed (SHA-256). When two media objects share identical chunk data, only one copy is stored on disk. A reference count tracks how many manifests reference each chunk. Chunks are only deleted from disk when their reference count drops to zero.

## API Reference

### REST Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `PUT` | `/v1/media/{key}` | Store media. Body: raw bytes. Headers: `Content-Type`, `X-KaidaDB-Meta-*` for custom metadata. Returns `201` with JSON. |
| `GET` | `/v1/media/{key}` | Stream media. Supports `Range: bytes=start-end` header. Returns `200` or `206 Partial Content`. |
| `HEAD` | `/v1/media/{key}` | Get metadata as response headers (`Content-Length`, `X-KaidaDB-Checksum`, `X-KaidaDB-Chunk-Count`, `X-KaidaDB-Meta-*`). |
| `DELETE` | `/v1/media/{key}` | Delete media. Returns `204` or `404`. |
| `GET` | `/v1/media?prefix=&limit=&cursor=` | List media with pagination. Returns JSON array. |
| `GET` | `/v1/meta/{key}` | Get metadata as JSON body. |
| `GET` | `/v1/health` | Health check. Returns `{"status": "ok", "version": "..."}`. |

### gRPC Service

Defined in `proto/kaidadb.proto`:

```protobuf
service KaidaDB {
    rpc StoreMedia(stream StoreMediaRequest) returns (StoreMediaResponse);
    rpc StreamMedia(StreamMediaRequest) returns (stream MediaChunk);
    rpc GetMediaMeta(GetMediaMetaRequest) returns (MediaMetadata);
    rpc DeleteMedia(DeleteMediaRequest) returns (DeleteMediaResponse);
    rpc ListMedia(ListMediaRequest) returns (ListMediaResponse);
    rpc HealthCheck(HealthCheckRequest) returns (HealthCheckResponse);
}
```

- **StoreMedia** — client-streaming: send a `StoreMediaHeader` first, then `chunk_data` messages
- **StreamMedia** — server-streaming: specify `key`, optional `offset` and `length` (0 = to end)
- **ListMedia** — cursor-based pagination with `prefix`, `limit`, `cursor` fields

## CLI Reference

```
kaidadb-cli [--addr <GRPC_ADDR>] <COMMAND>
```

Default address: `http://localhost:50051`

| Command | Usage | Description |
|---------|-------|-------------|
| `store` | `store <KEY> <FILE> [-c TYPE]` | Upload a file. Content type is auto-detected from extension if not specified. |
| `get` | `get <KEY> [-o FILE]` | Download media. Writes to stdout if no output file specified. |
| `meta` | `meta <KEY>` | Print metadata (size, chunks, content type, checksum, custom metadata). |
| `delete` | `delete <KEY>` | Delete media by key. |
| `list` | `list [-p PREFIX] [-l LIMIT]` | List media keys. Default limit: 100. |
| `health` | `health` | Server health check. |

**Auto-detected content types:** `.mp4`, `.mkv`, `.webm`, `.avi`, `.mp3`, `.flac`, `.wav`, `.ogg`, `.png`, `.jpg`/`.jpeg`, `.gif`, `.webp`. Falls back to `application/octet-stream`.

## Crate Reference

### kaidadb-common

Shared types used across all crates.

| Type | Description |
|------|-------------|
| `ChunkId` | 32-byte SHA-256 content hash. Methods: `from_data()`, `to_hex()`, `from_hex()`, `fan_out()`. |
| `ChunkLocation` | On-disk path + reference count for a chunk. |
| `MediaManifest` | Full metadata for a stored media object: key, ordered chunk list, size, content type, checksum, custom metadata, timestamps. |
| `KaidaDbConfig` | Top-level configuration with storage and cache sub-configs. Loads from TOML + env vars via figment. |
| `KaidaDbError` | Error enum: `NotFound`, `AlreadyExists`, `InvalidKey`, `Storage`, `ChunkIntegrity`, `Io`, `Serialization`, `InvalidChunkFormat`, `Config`, `Internal`. |

### kaidadb-storage

Core storage engine.

| Type | Description |
|------|-------------|
| `StorageEngine` | High-level facade. Methods: `open()`, `store()`, `store_with_metadata()`, `read()`, `read_range()`, `stream()`, `read_chunk()`, `get_manifest()`, `delete()`, `list()`. |
| `BlobStore` | Manages chunk files on disk with two-level hex fan-out. Methods: `write_chunk()`, `read_chunk()`, `delete_chunk()`, `chunk_exists()`. |
| `Index` | Append-only WAL with in-memory BTreeMap. Methods: manifest CRUD, chunk location CRUD, ref counting, `compact()`. |
| `chunk_format` | `encode_chunk()` / `decode_chunk()` — .kdc file format with magic, version, CRC32, payload. |

### kaidadb-cache

Size-bounded LRU chunk cache.

| Type | Description |
|------|-------------|
| `ChunkCache` | Thread-safe LRU cache keyed by `ChunkId`. Methods: `new(max_size)`, `get()`, `insert()`, `invalidate()`, `stats()`. |
| `CacheStats` | Snapshot: `hits`, `misses`, `current_size`, `max_size`, `entry_count`. |

Cache evicts least-recently-used chunks when `current_size + new_chunk > max_size`. Chunks exceeding `max_size` individually are not cached. The API layer checks the cache before hitting storage and populates it on cache misses.

### kaidadb-api

Dual gRPC + REST gateway.

| Type | Description |
|------|-------------|
| `KaidaDbGrpc` | tonic gRPC service implementation. Wraps `StorageEngine` + `ChunkCache`. |
| `rest::router()` | Returns an axum `Router` with all REST endpoints. Takes `AppState { engine, cache }`. |
| `proto::*` | Generated protobuf types from `proto/kaidadb.proto`. |

### kaidadb-server

Server binary. Starts both gRPC and REST servers concurrently. Graceful shutdown on SIGINT.

### kaidadb-cli

CLI client binary. Communicates with the server over gRPC.

## Testing

```bash
# Run all tests
cargo test --workspace

# Run only unit tests for a specific crate
cargo test -p kaidadb-storage

# Run integration tests
cargo test -p kaidadb-storage --test integration
```

### Test Coverage

| Crate | Tests | Coverage |
|-------|-------|----------|
| kaidadb-common | 5 | ChunkId hashing/encoding, config validation |
| kaidadb-storage (unit) | 19 | Chunk format encode/decode/corruption, blob store CRUD/dedup, index CRUD/ref-counting/persistence/compaction, engine store/read/range/stream/delete/list |
| kaidadb-storage (integration) | 15 | Full round-trips, range reads, overwrites, deduplication with ref-counted deletion, streaming, cache integration, persistence across restarts, large files |
| kaidadb-cache | 4 | Hit/miss tracking, LRU eviction, invalidation, size tracking |
| **Total** | **43** | |

## Roadmap

- **Phase 1** (current): Single-node storage engine with gRPC + REST APIs
- **Phase 2**: Cache warming, prefetch during streaming, cache benchmarks
- **Phase 3**: Distributed cluster — Raft consensus, consistent hashing, replication
- **Phase 4**: Failure detection, anti-entropy repair, metrics, TLS
- **Phase 5**: Operational tooling, hot-reload, backup/restore

## License

MIT
