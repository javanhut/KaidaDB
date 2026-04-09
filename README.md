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

## Installation

### Quick Install

```bash
# Clone and install all binaries (server, cli, tui) to ~/.local/bin
git clone <repo-url>
cd KaidaDB
./install.sh
```

This builds in release mode and installs `kaidadb-server`, `kaidadb-cli`, and `kaidadb-tui` to `~/.local/bin`, with a default config at `~/.config/kaidadb/config.toml` and data directory at `~/.local/share/kaidadb`.

### Install Options

```bash
# Custom install location
./install.sh --prefix /usr/local/bin --data /var/lib/kaidadb --config /etc/kaidadb

# Server only (no CLI or TUI)
./install.sh --server-only

# CLI only (for remote management)
./install.sh --cli-only

# Debug build
./install.sh --debug

# Skip config generation
./install.sh --no-config

# Uninstall binaries
./install.sh --uninstall
```

### Manual Build

```bash
cargo build --release

# Binaries are in target/release/
ls target/release/kaidadb-{server,cli,tui}
```

## Quick Start

### Run the Server

```bash
# Using the installed binary
kaidadb-server --config ~/.config/kaidadb/config.toml

# Or with cargo (development)
cargo run -p kaidadb-server -- --config config.toml

# With defaults (gRPC :50051, REST :8080, data in ./data)
cargo run -p kaidadb-server
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

### Interactive TUI

KaidaDB includes a terminal UI for browsing and managing your media library.

```bash
# Launch against default server
cargo run -p kaidadb-tui

# Custom server address
cargo run -p kaidadb-tui -- --addr http://localhost:50051
```

**Keybindings:**

| Key | Action |
|-----|--------|
| `j`/`k` or Up/Down | Navigate media list |
| `g`/`G` | Jump to first/last item |
| `Enter` | Full-screen detail view |
| `s` | Store media (prompts for key + file path) |
| `d` | Delete selected media (with confirmation) |
| `/` | Search/filter by key or content type |
| `n` | Next search match |
| `r` | Refresh list from server |
| `Tab` | Toggle active panel |
| `Esc` | Back/cancel |
| `q` | Quit |

## Organizing Media with Key Paths

KaidaDB keys are plain strings, but using `/`-delimited paths gives you a hierarchical namespace that keeps your media organized. Combined with **prefix queries**, this acts like a virtual directory tree — without any actual directory overhead.

### Key Naming Convention

Structure your keys as `type/title/season/episode`:

```
tv/breaking-bad/s01/e01-pilot
tv/breaking-bad/s01/e02-cats-in-the-bag
tv/breaking-bad/s02/e01-seven-thirty-seven

tv/the-wire/s01/e01-the-target
tv/the-wire/s01/e02-the-detail

movies/inception
movies/the-matrix
movies/the-matrix-reloaded

music/pink-floyd/dark-side-of-the-moon/01-speak-to-me
music/pink-floyd/dark-side-of-the-moon/02-breathe

podcasts/hardcore-history/ep01-alexander-vs-hitler
```

This solves the "every show has a pilot" problem — `tv/breaking-bad/s01/e01-pilot` and `tv/the-wire/s01/e01-the-target` are distinct keys even though both are pilot episodes, because the show name and season are part of the path.

### Browsing with Prefix Queries

The prefix-based list API lets you browse any level of the hierarchy:

```bash
# List all TV shows (just the top level)
kaidadb-cli list -p tv/

# List all seasons of Breaking Bad
kaidadb-cli list -p tv/breaking-bad/

# List all episodes in season 1
kaidadb-cli list -p tv/breaking-bad/s01/

# List all movies
kaidadb-cli list -p movies/

# List all Pink Floyd albums
kaidadb-cli list -p music/pink-floyd/
```

REST equivalent:

```bash
# Browse a show's seasons
curl "http://localhost:8080/v1/media?prefix=tv/breaking-bad/"

# Paginate through a large library
curl "http://localhost:8080/v1/media?prefix=tv/&limit=20"
curl "http://localhost:8080/v1/media?prefix=tv/&limit=20&cursor=tv/the-wire/s01/e02-the-detail"
```

In the TUI, press `/` and type a prefix like `tv/breaking-bad` to filter the list down to just that show.

### Recommended Key Patterns

| Media Type | Pattern | Example |
|-----------|---------|---------|
| TV shows | `tv/{show}/s{NN}/e{NN}-{slug}` | `tv/severance/s01/e01-good-news-about-hell` |
| Movies | `movies/{slug}` | `movies/blade-runner-2049` |
| Movie series | `movies/{series}/{slug}` | `movies/lord-of-the-rings/fellowship` |
| Music | `music/{artist}/{album}/{NN}-{track}` | `music/radiohead/ok-computer/01-airbag` |
| Podcasts | `podcasts/{show}/{slug}` | `podcasts/serial/s01e01` |
| Audio books | `audiobooks/{author}/{title}/{NN}` | `audiobooks/frank-herbert/dune/01` |
| Photos | `photos/{date}/{slug}` | `photos/2026-03/sunset-beach` |
| Surveillance | `cameras/{cam-id}/{date}/{time}` | `cameras/front-door/2026-03-16/14-30` |

### Tips

- **Use lowercase slugs** with hyphens — keys are case-sensitive, and consistency prevents duplicates like `TV/` vs `tv/`.
- **Put the most selective segment first** — `tv/show/season/episode` lets you query all episodes of a show efficiently. If you instead used `s01/tv/show/episode`, querying "all of Breaking Bad" would require scanning every season prefix.
- **Add metadata via headers** — instead of encoding everything in the key, attach structured data like resolution, language, or codec as custom metadata:
  ```bash
  curl -X PUT \
    -H "Content-Type: video/mp4" \
    -H "X-KaidaDB-Meta-resolution: 4k" \
    -H "X-KaidaDB-Meta-language: en" \
    -H "X-KaidaDB-Meta-codec: h265" \
    -T episode.mp4 \
    http://localhost:8080/v1/media/tv/severance/s01/e01-good-news-about-hell
  ```
- **Dedup works across keys** — if two episodes share identical intro sequences, the overlapping chunks are stored only once on disk regardless of key paths.

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
│   ├── kaidadb-cli/             # CLI client binary
│   └── kaidadb-tui/             # Interactive terminal UI
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

### kaidadb-tui

Interactive terminal UI built with ratatui. Split-pane layout with media browser, detail panel, search/filter, store, and delete dialogs. Communicates with the server over gRPC.

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

## Deployment

### Running as a systemd Service

Create a service file at `/etc/systemd/system/kaidadb.service`:

```ini
[Unit]
Description=KaidaDB Media Database
After=network.target

[Service]
Type=simple
User=kaidadb
Group=kaidadb
ExecStart=/usr/local/bin/kaidadb-server --config /etc/kaidadb/config.toml
Restart=on-failure
RestartSec=5

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/kaidadb

[Install]
WantedBy=multi-user.target
```

Set it up:

```bash
# Install to system paths
sudo ./install.sh --prefix /usr/local/bin --data /var/lib/kaidadb --config /etc/kaidadb

# Create a dedicated user
sudo useradd -r -s /usr/sbin/nologin -d /var/lib/kaidadb kaidadb
sudo chown -R kaidadb:kaidadb /var/lib/kaidadb

# Edit config as needed
sudo vim /etc/kaidadb/config.toml

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable --now kaidadb

# Check status
sudo systemctl status kaidadb
journalctl -u kaidadb -f
```

### Running with Docker

```dockerfile
FROM rust:1.82-slim AS builder
WORKDIR /build
COPY . .
RUN apt-get update && apt-get install -y protobuf-compiler && \
    cargo build --release -p kaidadb-server

FROM debian:bookworm-slim
COPY --from=builder /build/target/release/kaidadb-server /usr/local/bin/
COPY config.toml /etc/kaidadb/config.toml
EXPOSE 50051 8080
VOLUME /data
ENV KAIDADB_DATA_DIR=/data
CMD ["kaidadb-server", "--config", "/etc/kaidadb/config.toml"]
```

```bash
# Build and run
docker build -t kaidadb .
docker run -d -p 8080:8080 -p 50051:50051 -v kaidadb-data:/data --name kaidadb kaidadb

# Verify
curl http://localhost:8080/v1/health
```

### Remote CLI Access

Install the CLI on any machine to manage a remote server:

```bash
# Install CLI only
./install.sh --cli-only --no-config

# Point to the remote server
kaidadb-cli --addr http://your-server:50051 health
kaidadb-cli --addr http://your-server:50051 list
kaidadb-cli --addr http://your-server:50051 store my-video ./video.mp4
```

### Using with Reelscape (OSSFlix)

KaidaDB integrates with [Reelscape](../OSSFlix/) as an optional media storage backend. Once KaidaDB is running:

1. Open Reelscape Settings and enter the KaidaDB REST URL (e.g., `http://localhost:8080` or `http://your-server:8080`)
2. Click **Test** to verify the connection
3. Ingest media via the API:
   ```bash
   curl -X POST http://localhost:3000/api/kaidadb/ingest \
     -H "Content-Type: application/json" \
     -d '{"src": "/media/movies/your_movie/movie.mp4"}'
   ```
4. Playback will automatically stream from KaidaDB with full seeking support

### Production Checklist

- [ ] Use a dedicated data partition with sufficient storage
- [ ] Set `KAIDADB_DATA_DIR` to a path on the data partition
- [ ] Tune `cache.max_size` based on available RAM (recommended: 25-50% of free memory)
- [ ] Increase `storage.chunk_size` for large files (4-8 MiB reduces index overhead)
- [ ] Set up log rotation for `journalctl` or redirect logs
- [ ] Back up `$DATA_DIR/index/` periodically (the WAL is the source of truth)
- [ ] Use a reverse proxy (nginx/caddy) for TLS termination on the REST port
- [ ] Firewall: expose only the REST port (8080) publicly; keep gRPC (50051) internal

## Roadmap

- **Phase 1** (current): Single-node storage engine with gRPC + REST APIs
- **Phase 2**: Cache warming, prefetch during streaming, cache benchmarks
- **Phase 3**: Distributed cluster — Raft consensus, consistent hashing, replication
- **Phase 4**: Failure detection, anti-entropy repair, metrics, TLS
- **Phase 5**: Operational tooling, hot-reload, backup/restore

## License

MIT
