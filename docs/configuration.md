# Configuration

## How Configuration Works

KaidaDB loads configuration from three sources, in this priority order:

1. **Environment variables** (highest priority) — override everything
2. **TOML config file** — specified via `--config` flag
3. **Built-in defaults** — used when nothing else is specified

This means you can run KaidaDB with zero configuration (all defaults), use a config file for persistent settings, and override individual values with environment variables for quick changes or container deployments.

## Config File

Pass a config file path when starting the server:

```bash
kaidadb-server --config /path/to/config.toml
```

### Full Reference

```toml
# Where KaidaDB stores its data (chunks and index)
data_dir = "./data"

# Network addresses
grpc_addr = "0.0.0.0:50051"    # gRPC API (used by CLI and TUI)
rest_addr = "0.0.0.0:8080"     # REST API (used by web apps, curl, media players)

[storage]
# Size of each chunk in bytes.
# Larger chunks = fewer chunks per file = smaller index, but less dedup granularity.
# Valid range: 1,048,576 (1 MiB) to 16,777,216 (16 MiB)
chunk_size = 2097152  # 2 MiB

[cache]
# Maximum memory for the chunk cache in bytes.
# Larger cache = more media served from RAM = faster playback.
max_size = 536870912  # 512 MiB

# Number of chunks to read ahead during sequential streaming.
# Higher values reduce latency for sequential playback but use more memory.
prefetch_window = 3

# Whether to cache the first N chunks when media is stored.
# Useful if media is typically played back shortly after upload.
warm_on_write = false

[streaming]
# Default segment duration in seconds for HLS/DASH playlist generation.
target_duration = 4.0

# Base URL prepended to segment URLs in playlists.
# Leave empty for relative paths (works when KaidaDB serves directly).
# Set to your CDN URL if using a CDN in front of KaidaDB.
base_url = ""

# Key prefix where streaming content is stored.
stream_prefix = "streams/"

# Whether to include #EXT-X-ENDLIST in HLS playlists.
# true = VOD (finished content). false = live-like (playlist can grow).
vod_mode = true
```

## Environment Variables

Every config option can be set via environment variable with the `KAIDADB_` prefix. Nested keys use underscores.

| Variable | Default | Description |
|----------|---------|-------------|
| `KAIDADB_DATA_DIR` | `./data` | Data storage directory |
| `KAIDADB_GRPC_ADDR` | `0.0.0.0:50051` | gRPC listen address |
| `KAIDADB_REST_ADDR` | `0.0.0.0:8080` | REST listen address |
| `KAIDADB_STORAGE_CHUNK_SIZE` | `2097152` | Chunk size in bytes |
| `KAIDADB_CACHE_MAX_SIZE` | `536870912` | Max cache memory in bytes |
| `KAIDADB_CACHE_PREFETCH_WINDOW` | `3` | Chunks to prefetch ahead |
| `KAIDADB_CACHE_WARM_ON_WRITE` | `false` | Cache chunks on write |
| `KAIDADB_STREAMING_TARGET_DURATION` | `4.0` | Default segment duration |
| `KAIDADB_STREAMING_BASE_URL` | `""` | Base URL for playlist segment URLs |
| `KAIDADB_STREAMING_STREAM_PREFIX` | `streams/` | Key prefix for streams |
| `KAIDADB_STREAMING_VOD_MODE` | `true` | Include EXT-X-ENDLIST in HLS |

**Example: Override with environment variables**

```bash
KAIDADB_DATA_DIR=/mnt/media/kaidadb \
KAIDADB_CACHE_MAX_SIZE=1073741824 \
kaidadb-server --config config.toml
```

**Docker example:**

```bash
docker run -d \
  -e KAIDADB_DATA_DIR=/data \
  -e KAIDADB_CACHE_MAX_SIZE=1073741824 \
  -e KAIDADB_REST_ADDR=0.0.0.0:8080 \
  -v kaidadb-data:/data \
  -p 8080:8080 -p 50051:50051 \
  kaidadb
```

## Server Password

KaidaDB auto-generates a unique password for each server instance to protect remote access. This is not a config file option — it's managed separately.

### Where to Find the Password

The password is **printed to the server log output on first start only**. Look for the line:

```
INFO kaidadb:   Generated new server key: aB3xK9mP...
```

If you started KaidaDB as a service, check the logs:

```bash
# systemd
journalctl --user -u kaidadb | grep "server key"

# kaidadb-ctl
grep "server key" ~/.local/state/kaidadb/kaidadb.log
```

The password is **not stored on disk** — only its SHA-256 hash is saved at `{data_dir}/.server_key`. You cannot recover the plaintext from that file.

### If You Lost the Password

Regenerate a new one:

```bash
kaidadb-server --regenerate-key --config /path/to/config.toml
# Prints: New server key: xY7pQ2wR...
```

This overwrites the old hash. Any clients using the old password will be rejected until updated.

### How It Works

- On first server start, a random 32-character alphanumeric password is generated
- The SHA-256 hash is stored in `{data_dir}/.server_key`
- Local connections (127.0.0.1 / ::1) bypass auth entirely
- Remote connections must include the password on every request (both REST and gRPC)

### Client Usage

```bash
# CLI
kaidadb-cli --addr http://remote:50051 --server-pass <password> list

# TUI
kaidadb-tui --addr http://remote:50051 --server-pass <password>

# REST
curl -H "X-Server-Pass: <password>" http://remote:8080/v1/health
```

## Logging

KaidaDB uses the `RUST_LOG` environment variable for log level control:

```bash
# Default (info level for kaidadb and tower_http)
RUST_LOG=kaidadb=info,tower_http=info kaidadb-server

# Debug logging (verbose)
RUST_LOG=kaidadb=debug kaidadb-server

# Trace logging (very verbose, includes every chunk read)
RUST_LOG=kaidadb=trace kaidadb-server

# Quiet (warnings and errors only)
RUST_LOG=kaidadb=warn kaidadb-server
```

## Tuning Guide

### Chunk Size

The chunk size controls how large each piece is when a file is split up. It affects several things:

| Chunk Size | Index Size | Dedup Granularity | Range Request Overhead |
|------------|-----------|-------------------|----------------------|
| 1 MiB | Larger (more chunks per file) | Finer (more dedup opportunities) | Lower (smaller reads) |
| 2 MiB (default) | Balanced | Balanced | Balanced |
| 4 MiB | Smaller | Coarser | Moderate |
| 8-16 MiB | Smallest | Coarsest | Higher (reads more data per chunk) |

**Recommendations:**
- **Small files** (< 10 MB each, like photos or short audio clips): 1 MiB chunks
- **Mixed media** (videos, music, podcasts): 2 MiB (default) — good all-around
- **Large files** (4K video, uncompressed audio, > 1 GB): 4-8 MiB — reduces index size significantly

### Cache Size

The cache keeps recently-accessed chunks in memory. A larger cache means more data served from RAM instead of disk.

**Recommendations:**
- **Minimal system** (Raspberry Pi, 1 GB RAM): 64-128 MiB
- **Personal server** (4-8 GB RAM): 512 MiB - 1 GiB
- **Dedicated media server** (16+ GB RAM): 2-4 GiB
- **Rule of thumb**: 25-50% of available free memory

```toml
[cache]
max_size = 1073741824  # 1 GiB
```

### Prefetch Window

Controls how many chunks are read ahead during sequential playback. Higher values reduce stutter but use more memory and I/O bandwidth.

| Value | Behavior |
|-------|----------|
| 1 | Minimal prefetch — good for random access patterns |
| 3 (default) | Good for most streaming |
| 5-8 | Aggressive — good for high-bitrate 4K video on fast storage |

### Warm on Write

When `warm_on_write = true`, the first `prefetch_window` chunks of newly-stored media are placed in the cache immediately. Enable this if you expect media to be played back soon after upload (e.g., a live recording workflow).

## Example Configurations

### Raspberry Pi (Minimal Resources)

```toml
data_dir = "/media/usb/kaidadb"
grpc_addr = "0.0.0.0:50051"
rest_addr = "0.0.0.0:8080"

[storage]
chunk_size = 2097152  # 2 MiB

[cache]
max_size = 67108864  # 64 MiB (Pi has limited RAM)
prefetch_window = 2
warm_on_write = false
```

### Home Media Server

```toml
data_dir = "/mnt/media/kaidadb"
grpc_addr = "0.0.0.0:50051"
rest_addr = "0.0.0.0:8080"

[storage]
chunk_size = 4194304  # 4 MiB (larger files, fewer chunks)

[cache]
max_size = 2147483648  # 2 GiB
prefetch_window = 5
warm_on_write = false

[streaming]
target_duration = 4.0
vod_mode = true
```

### Music Streaming Server

```toml
data_dir = "/var/lib/kaidadb"
grpc_addr = "127.0.0.1:50051"  # Local only (CLI/TUI access)
rest_addr = "0.0.0.0:8080"     # Exposed to network

[storage]
chunk_size = 1048576  # 1 MiB (music files are smaller, finer dedup)

[cache]
max_size = 536870912  # 512 MiB
prefetch_window = 3
warm_on_write = true  # Songs are often played right after upload

[streaming]
target_duration = 4.0
vod_mode = true
```

### CDN Origin Server

```toml
data_dir = "/data/kaidadb"
grpc_addr = "127.0.0.1:50051"  # Internal only
rest_addr = "0.0.0.0:8080"

[storage]
chunk_size = 4194304  # 4 MiB

[cache]
max_size = 4294967296  # 4 GiB (CDN reduces repeat requests)
prefetch_window = 3
warm_on_write = false

[streaming]
target_duration = 4.0
base_url = "https://cdn.example.com"  # CDN URL in playlists
vod_mode = true
```

## Production Checklist

- [ ] Use a dedicated data partition with sufficient storage
- [ ] Set `data_dir` to a path on the data partition
- [ ] Tune `cache.max_size` based on available RAM (25-50% of free memory)
- [ ] Increase `storage.chunk_size` for large files (4-8 MiB reduces index overhead)
- [ ] **Save the server password** from first boot — store it securely
- [ ] Back up `$DATA_DIR/.server_key` along with `$DATA_DIR/index/` periodically
- [ ] Set up log rotation for `journalctl` or kaidadb-ctl logs
- [ ] Use a reverse proxy (nginx/caddy) for TLS termination on the REST port
- [ ] Firewall: expose only the REST port (8080) publicly; keep gRPC (50051) internal
- [ ] Set `RUST_LOG=kaidadb=info` for production (avoid debug/trace in production)
