# Getting Started with KaidaDB

## What Is KaidaDB?

KaidaDB is a media database. Think of it as a personal hard drive you can talk to over the network — you put files in, you get files out, and it handles all the messy details of storing them efficiently, streaming them smoothly, and keeping them safe.

It's built for **self-hosting**. You run it on your own hardware — a server in your closet, a Raspberry Pi, a VPS — and it stores your videos, music, podcasts, photos, or any other media. No cloud accounts, no subscriptions, no vendor lock-in.

KaidaDB comes with three tools:

- **kaidadb-server** — The database itself. Runs in the background and listens for requests.
- **kaidadb-cli** — A command-line tool for storing, retrieving, and managing media.
- **kaidadb-tui** — An interactive terminal interface for browsing your media library.

## Running the Server

### From a Release Build

If you installed via `./install.sh`:

```bash
kaidadb-server --config ~/.config/kaidadb/config.toml
```

### During Development

```bash
# With the default config file in the repo
cargo run -p kaidadb-server -- --config config.toml

# With all defaults (gRPC on :50051, REST on :8080, data in ./data)
cargo run -p kaidadb-server
```

### What Happens on Startup

When the server starts, it:

1. Loads configuration from your config file and any `KAIDADB_*` environment variables
2. Opens (or creates) the data directory
3. Replays the index log to rebuild its in-memory state
4. Starts listening on two ports:
   - **Port 50051** — gRPC (used by the CLI and TUI)
   - **Port 8080** — REST API (used by web apps, curl, media players)

You'll see output like:

```
INFO kaidadb: starting KaidaDB server config=KaidaDbConfig { ... }
INFO kaidadb: server listening grpc_addr=0.0.0.0:50051 rest_addr=0.0.0.0:8080
```

Press `Ctrl-C` to shut down gracefully.

## Storing Your First File

### Using the CLI

```bash
# Store a video
kaidadb-cli store movies/inception ./inception.mp4

# Store a song
kaidadb-cli store music/radiohead/creep ./creep.mp3

# Store with a custom content type
kaidadb-cli store podcasts/ep01 ./episode.m4a --content-type audio/mp4
```

The CLI auto-detects content types from file extensions (`.mp4`, `.mp3`, `.flac`, `.wav`, `.ogg`, `.mkv`, `.webm`, `.avi`, `.png`, `.jpg`, `.gif`, `.webp`). If it can't detect the type, it defaults to `application/octet-stream`.

### Using the REST API

```bash
# Store a file with curl
curl -X PUT \
  -H "Content-Type: video/mp4" \
  -T ./inception.mp4 \
  http://localhost:8080/v1/media/movies/inception

# Store with custom metadata
curl -X PUT \
  -H "Content-Type: audio/flac" \
  -H "X-KaidaDB-Meta-artist: Radiohead" \
  -H "X-KaidaDB-Meta-album: Pablo Honey" \
  -T ./creep.flac \
  http://localhost:8080/v1/media/music/radiohead/creep
```

## Retrieving Media

### Download a File

```bash
# CLI
kaidadb-cli get movies/inception -o inception.mp4

# REST
curl http://localhost:8080/v1/media/movies/inception -o inception.mp4
```

### Stream a Portion (Range Request)

Range requests let you seek to any point in a file without downloading the whole thing. This is how video and audio players handle scrubbing.

```bash
# First 1 MiB
curl -H "Range: bytes=0-1048575" http://localhost:8080/v1/media/movies/inception -o header.bin

# Last 500 bytes
curl -H "Range: bytes=-500" http://localhost:8080/v1/media/movies/inception -o tail.bin
```

### Check Metadata

```bash
# CLI
kaidadb-cli meta movies/inception
# Output:
#   Key:          movies/inception
#   Size:         1,234,567,890 bytes
#   Chunks:       589
#   Content-Type: video/mp4
#   Checksum:     a1b2c3d4...

# REST (as JSON)
curl http://localhost:8080/v1/meta/movies/inception

# REST (as headers only, no body)
curl -I http://localhost:8080/v1/media/movies/inception
```

### List and Browse

```bash
# List everything
kaidadb-cli list

# List with a prefix filter
kaidadb-cli list -p movies/

# List with a limit
kaidadb-cli list -p tv/breaking-bad/ -l 10

# REST equivalent
curl "http://localhost:8080/v1/media?prefix=movies/&limit=50"

# Paginate with cursor
curl "http://localhost:8080/v1/media?prefix=tv/&limit=20&cursor=tv/the-wire/s01/e02"
```

### Delete and Rename

```bash
# Delete
kaidadb-cli delete movies/inception

# Rename (REST only)
curl -X POST http://localhost:8080/v1/media/rename \
  -H "Content-Type: application/json" \
  -d '{"from_key": "movies/old-name", "to_key": "movies/new-name"}'
```

## Using the TUI

The terminal UI gives you a visual way to browse, search, upload, and manage your media library.

### Launch

```bash
# Default server (localhost:50051)
kaidadb-tui

# Or during development
cargo run -p kaidadb-tui

# Custom server address
kaidadb-tui --addr http://your-server:50051
```

### Keybindings

| Key | Action |
|-----|--------|
| `j` / `k` or Arrow keys | Navigate up/down |
| `g` / `G` | Jump to first / last item |
| `Enter` or `l` | Open detail view / enter directory |
| `Backspace` or `Left` | Go up one directory |
| `s` | Store (upload) a file |
| `d` | Delete selected media (asks for confirmation) |
| `m` | Rename selected media |
| `M` | Create a new directory |
| `/` | Search / filter |
| `n` | Next search match |
| `r` | Refresh the media list |
| `Tab` | Toggle between list and detail panels |
| `Esc` | Cancel / go back |
| `q` | Quit |

### Walkthrough

1. **Browsing** — The TUI shows your media as a virtual directory tree. Keys with `/` separators are displayed as folders. Press `Enter` to drill into a folder, `Backspace` to go up.

2. **Searching** — Press `/`, type a search term, and press `Enter`. The list filters to matching items. Press `n` to jump to the next match. Press `Esc` to clear the search.

3. **Uploading** — Press `s`. You'll be prompted for a key (the path in KaidaDB) and then a local file path. The TUI uses a file browser so you can navigate your local filesystem.

4. **Deleting** — Select a media item, press `d`, then confirm with `y`. This permanently removes the media and cleans up any chunks that are no longer referenced by other media.

5. **Renaming** — Select a media item, press `m`, edit the key, and press `Enter`.

## Running as a Service

For production or always-on setups, you'll want KaidaDB to start automatically and restart on failure.

### Install with Service Support

```bash
./install.sh --service
```

This auto-detects your init system and installs the appropriate service configuration.

### systemd (Arch, Ubuntu, Debian, Fedora, etc.)

A **user service** is installed — no root required:

```bash
# Start
systemctl --user start kaidadb

# Stop
systemctl --user stop kaidadb

# View logs
journalctl --user -u kaidadb -f

# Check status
systemctl --user status kaidadb

# Start on boot (even when not logged in)
loginctl enable-linger $(whoami)
```

### OpenRC (Alpine, Gentoo, Artix)

The installer generates an init script and prints the `sudo` commands to install it:

```bash
sudo cp /tmp/kaidadb.openrc /etc/init.d/kaidadb
sudo chmod +x /etc/init.d/kaidadb
sudo rc-update add kaidadb default
sudo rc-service kaidadb start
```

### runit (Void, Artix with runit)

The installer creates a run script and prints the `sudo` commands to link it:

```bash
sudo mkdir -p /etc/sv/kaidadb
sudo cp /tmp/kaidadb-run /etc/sv/kaidadb/run
sudo ln -s /etc/sv/kaidadb /var/service/
```

### kaidadb-ctl (Any Distro)

`kaidadb-ctl` is a portable daemon manager that works everywhere. It's always installed alongside `--service` as a fallback:

```bash
kaidadb-ctl start      # Start in background
kaidadb-ctl stop       # Graceful shutdown (SIGTERM, then SIGKILL after 10s)
kaidadb-ctl restart    # Stop + start
kaidadb-ctl status     # Check if running
kaidadb-ctl logs       # Tail the log file
```

Logs are written to `~/.local/state/kaidadb/kaidadb.log`.

### System-Wide Service (Production)

For dedicated servers running KaidaDB as a system service:

```bash
# Install to system paths
sudo ./install.sh --prefix /usr/local/bin --data /var/lib/kaidadb --config /etc/kaidadb --service

# Create a dedicated user
sudo useradd -r -s /usr/sbin/nologin -d /var/lib/kaidadb kaidadb
sudo chown -R kaidadb:kaidadb /var/lib/kaidadb

# Edit the systemd unit to set User=kaidadb, then:
sudo systemctl enable --now kaidadb
```

## Running with Docker

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

## Remote CLI Access

You can install just the CLI on any machine to manage a remote KaidaDB server:

```bash
# Install CLI only (no server, no config generation)
./install.sh --cli-only --no-config

# Point to your remote server
kaidadb-cli --addr http://your-server:50051 health
kaidadb-cli --addr http://your-server:50051 list
kaidadb-cli --addr http://your-server:50051 store my-video ./video.mp4
```

## Using with Reelscape

KaidaDB integrates with [Reelscape](https://github.com/javanhut/Reelscape) as a media storage backend:

1. Start KaidaDB
2. In Reelscape Settings, enter the KaidaDB REST URL (e.g., `http://localhost:8080`)
3. Click **Test** to verify the connection
4. Ingest media:
   ```bash
   curl -X POST http://localhost:3000/api/kaidadb/ingest \
     -H "Content-Type: application/json" \
     -d '{"src": "/media/movies/your_movie/movie.mp4"}'
   ```
5. Playback streams directly from KaidaDB with full seeking support

## Next Steps

- [Architecture](./architecture.md) — How KaidaDB works under the hood
- [Streaming Guide](./streaming-guide.md) — Set up HLS/DASH video and music streaming
- [API Reference](./api-reference.md) — Complete REST and gRPC endpoint documentation
- [Configuration](./configuration.md) — All config options and tuning advice
- [CLI and TUI](./cli-and-tui.md) — Detailed command reference and media organization
- [When to Use KaidaDB](./when-to-use-kaidadb.md) — Strengths, limitations, and use cases
