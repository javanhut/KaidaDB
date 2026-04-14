# KaidaDB

A self-hosted media database built in Rust. Store and stream video, audio, images, and any binary media with content-addressed chunk deduplication, LRU caching, and dual gRPC/REST APIs.

- **Content-addressed storage** — chunks are SHA-256 hashed; identical data is stored once
- **Streaming delivery** — memory-mapped I/O with backpressure-aware async channels
- **HLS/DASH support** — adaptive bitrate streaming with on-the-fly playlist generation
- **Range requests** — seek to any byte offset with O(1) chunk lookup
- **LRU caching** — size-bounded chunk cache keeps hot media in memory
- **Dual API** — gRPC for performance, REST with Range header support for compatibility
- **Zero external dependencies** — no cloud services, no external databases, one binary

## Installation

### Quick Install

```bash
git clone <repo-url>
cd KaidaDB
./install.sh
```

Builds in release mode and installs `kaidadb-server`, `kaidadb-cli`, and `kaidadb-tui` to `~/.local/bin`, with a default config at `~/.config/kaidadb/config.toml` and data directory at `~/.local/share/kaidadb`.

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

# Install with service (auto-start on boot)
./install.sh --service

# Uninstall binaries and services
./install.sh --uninstall
```

### Manual Build

```bash
cargo build --release

# Binaries
ls target/release/kaidadb-{server,cli,tui}
```

### Requirements

- **Rust** 1.70+ (for building)
- **protoc** (Protocol Buffers compiler, for gRPC code generation)

## What's Included

| Binary | Description |
|--------|-------------|
| `kaidadb-server` | The database server. Runs gRPC (port 50051) and REST (port 8080) APIs. |
| `kaidadb-cli` | Command-line client. Store, retrieve, list, and delete media over gRPC. |
| `kaidadb-tui` | Interactive terminal UI. Browse, search, upload, and manage your media library. |

## Quick Start

```bash
# Start the server
kaidadb-server --config ~/.config/kaidadb/config.toml

# Store a file
kaidadb-cli store movies/inception ./inception.mp4

# Retrieve it
kaidadb-cli get movies/inception -o output.mp4

# List media
kaidadb-cli list -p movies/

# Launch the TUI
kaidadb-tui
```

## Documentation

| Document | Description |
|----------|-------------|
| [Getting Started](docs/getting-started.md) | First steps: running the server, storing files, using the TUI, deploying as a service |
| [Architecture](docs/architecture.md) | How KaidaDB works under the hood: storage engine, chunking, deduplication, caching |
| [Streaming Guide](docs/streaming-guide.md) | Set up HLS/DASH video and music streaming with FFmpeg |
| [API Reference](docs/api-reference.md) | Complete REST and gRPC endpoint documentation |
| [Configuration](docs/configuration.md) | All config options, environment variables, and tuning advice |
| [CLI and TUI](docs/cli-and-tui.md) | Command reference, keybindings, and media organization patterns |
| [When to Use KaidaDB](docs/when-to-use-kaidadb.md) | Strengths, limitations, and guidance on when KaidaDB is the right tool |

## License

MIT
