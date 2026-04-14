# CLI and TUI Reference

## CLI (`kaidadb-cli`)

The CLI communicates with the KaidaDB server over gRPC. It's the fastest way to store, retrieve, and manage media from the command line.

### Usage

```
kaidadb-cli [--addr <GRPC_ADDRESS>] <COMMAND>
```

Default address: `http://localhost:50051`

### Commands

#### `store` — Upload a File

```bash
kaidadb-cli store <KEY> <FILE> [--content-type TYPE]
```

Uploads a local file to KaidaDB under the given key. The content type is auto-detected from the file extension unless you specify one.

```bash
# Auto-detected content type
kaidadb-cli store movies/inception ./inception.mp4

# Explicit content type
kaidadb-cli store podcasts/ep01 ./episode.m4a --content-type audio/mp4

# Hierarchical key
kaidadb-cli store tv/breaking-bad/s01/e01 ./pilot.mkv
```

**Auto-detected content types:**

| Extension | Content Type |
|-----------|-------------|
| `.mp4` | `video/mp4` |
| `.mkv` | `video/x-matroska` |
| `.webm` | `video/webm` |
| `.avi` | `video/x-msvideo` |
| `.mp3` | `audio/mpeg` |
| `.flac` | `audio/flac` |
| `.wav` | `audio/wav` |
| `.ogg` | `audio/ogg` |
| `.png` | `image/png` |
| `.jpg` / `.jpeg` | `image/jpeg` |
| `.gif` | `image/gif` |
| `.webp` | `image/webp` |
| Other | `application/octet-stream` |

The CLI streams the file in 2 MiB chunks over gRPC, so it handles files of any size without loading them entirely into memory.

#### `get` — Download a File

```bash
kaidadb-cli get <KEY> [-o FILE]
```

Downloads media from KaidaDB. If no output file is specified, the data is written to stdout (useful for piping).

```bash
# Save to file
kaidadb-cli get movies/inception -o inception.mp4

# Pipe to ffplay
kaidadb-cli get music/song.mp3 | ffplay -

# Pipe to another tool
kaidadb-cli get data/export.json | jq '.results'
```

#### `meta` — Show Metadata

```bash
kaidadb-cli meta <KEY>
```

Prints metadata about a stored media object without downloading it.

```bash
kaidadb-cli meta movies/inception
```

Output:

```
Key:          movies/inception
Size:         1,234,567,890 bytes
Chunks:       589
Content-Type: video/mp4
Checksum:     a1b2c3d4e5f6...
Metadata:
  resolution: 4k
  language:   en
Created:      2026-03-16T14:30:00Z
Updated:      2026-03-16T14:30:00Z
```

#### `delete` — Remove Media

```bash
kaidadb-cli delete <KEY>
```

Deletes a media object. Chunks that are no longer referenced by any other media are removed from disk.

```bash
kaidadb-cli delete movies/inception
```

#### `list` — List Media

```bash
kaidadb-cli list [-p PREFIX] [-l LIMIT]
```

Lists media keys. Use the prefix flag to filter by path and the limit flag to control how many results are returned.

```bash
# List everything (up to 100)
kaidadb-cli list

# List all movies
kaidadb-cli list -p movies/

# List Breaking Bad season 1
kaidadb-cli list -p tv/breaking-bad/s01/

# Show first 10 results
kaidadb-cli list -l 10
```

#### `health` — Check Server Status

```bash
kaidadb-cli health
```

```
Status: ok
Version: 0.1.0
```

### Remote Server Access

Point the CLI at any reachable KaidaDB server:

```bash
kaidadb-cli --addr http://192.168.1.50:50051 list
kaidadb-cli --addr http://my-server.local:50051 store backup/photos ./photos.tar
```

---

## Organizing Media with Key Paths

KaidaDB keys are plain strings, but using `/`-delimited paths creates a virtual directory tree. This is powerful because:

- **Prefix queries** act like "list files in a folder" — `list -p tv/breaking-bad/` shows all episodes
- **The TUI** renders keys as a browsable directory tree
- **No actual directories** exist on disk — it's just a naming convention, so there's no overhead

### Recommended Key Patterns

| Media Type | Pattern | Example |
|-----------|---------|---------|
| TV shows | `tv/{show}/s{NN}/e{NN}-{slug}` | `tv/severance/s01/e01-good-news-about-hell` |
| Movies | `movies/{slug}` | `movies/blade-runner-2049` |
| Movie series | `movies/{series}/{slug}` | `movies/lord-of-the-rings/fellowship` |
| Music | `music/{artist}/{album}/{NN}-{track}` | `music/radiohead/ok-computer/01-airbag` |
| Podcasts | `podcasts/{show}/{slug}` | `podcasts/serial/s01e01` |
| Audiobooks | `audiobooks/{author}/{title}/{NN}` | `audiobooks/frank-herbert/dune/01` |
| Photos | `photos/{date}/{slug}` | `photos/2026-03/sunset-beach` |
| Surveillance | `cameras/{cam-id}/{date}/{time}` | `cameras/front-door/2026-03-16/14-30` |

### Tips

- **Use lowercase slugs with hyphens** — keys are case-sensitive. Consistency prevents duplicates like `TV/` vs `tv/`.

- **Put the most selective segment first** — `tv/show/season/episode` lets you query all episodes of a show with a single prefix. If you used `s01/tv/show/episode`, finding "all of Breaking Bad" would require scanning every season prefix.

- **Attach metadata with headers, not in the key** — instead of `movies/inception-4k-en-h265`, store as `movies/inception` with metadata:
  ```bash
  curl -X PUT \
    -H "X-KaidaDB-Meta-resolution: 4k" \
    -H "X-KaidaDB-Meta-language: en" \
    -H "X-KaidaDB-Meta-codec: h265" \
    -T movie.mp4 \
    http://localhost:8080/v1/media/movies/inception
  ```

- **Dedup works across keys** — if two episodes share the same intro sequence, the overlapping chunks are stored only once, regardless of key paths.

---

## TUI (`kaidadb-tui`)

The terminal user interface provides a visual way to browse, search, upload, and manage your media library. It connects to a KaidaDB server over gRPC.

### Launch

```bash
# Default server (localhost:50051)
kaidadb-tui

# During development
cargo run -p kaidadb-tui

# Custom server address
kaidadb-tui --addr http://your-server:50051
```

### Interface Layout

The TUI has two panels:

- **Left panel (List)** — Shows your media as a directory tree. Keys with `/` separators are displayed as folders that you can drill into.
- **Right panel (Detail)** — Shows metadata for the selected item (size, content type, checksum, custom metadata, timestamps).

Press `Tab` to switch focus between panels.

### Keybindings

#### Normal Mode

| Key | Action |
|-----|--------|
| `j` / `k` or Up / Down | Navigate up/down in the list |
| `g` / `G` or Home / End | Jump to first / last item |
| `Enter` / `l` / Right | Open detail view or enter a directory |
| `Backspace` / Left | Go up one directory level |
| `s` | Store (upload) a file |
| `d` | Delete selected media |
| `m` | Rename selected media |
| `M` | Create a new directory (virtual — stores a placeholder) |
| `/` | Open search/filter |
| `n` | Jump to next search match |
| `r` | Refresh media list from server |
| `Tab` | Toggle active panel (List / Detail) |
| `Esc` | Cancel current action / go back |
| `q` / `Ctrl-C` | Quit (in root view) / go up (when browsing) |

#### Search Mode

After pressing `/`:

| Key | Action |
|-----|--------|
| Type | Enter search text |
| `Enter` | Apply search filter |
| `Esc` | Cancel search |

#### Delete Confirmation

After pressing `d`:

| Key | Action |
|-----|--------|
| `y` | Confirm deletion |
| `n` / `Esc` | Cancel |

#### File Browser (Upload)

After pressing `s`, you'll be prompted for a key, then enter the file browser:

| Key | Action |
|-----|--------|
| Up / Down | Navigate files |
| `Enter` | Select file / enter directory |
| `Backspace` | Go up one directory |
| `Esc` | Cancel upload |

### Walkthrough

#### Browsing Your Library

When you launch the TUI, it fetches the media list and displays it as a directory tree. If you have keys like:

```
tv/breaking-bad/s01/e01
tv/breaking-bad/s01/e02
movies/inception
music/radiohead/ok-computer/01-airbag
```

You'll see top-level folders: `tv/`, `movies/`, `music/`. Press `Enter` on `tv/` to see `breaking-bad/`, then again for `s01/`, then the individual episodes.

Press `Backspace` to go back up.

#### Searching

Press `/` and type a search term (e.g., `radiohead`). The list filters to show only matching items. Press `n` to cycle through matches. Press `Esc` to clear the filter and show everything again.

#### Uploading a File

1. Press `s`
2. Enter the key you want to store the file under (e.g., `music/new-song`)
3. The file browser opens — navigate your local filesystem to find the file
4. Press `Enter` on the file to upload it
5. The TUI shows upload progress

#### Deleting Media

1. Navigate to the item you want to delete
2. Press `d`
3. A confirmation prompt appears — press `y` to confirm or `n` to cancel
4. The item is removed and the list refreshes

#### Renaming Media

1. Navigate to the item
2. Press `m`
3. Edit the key (the cursor starts at the end of the current key)
4. Press `Enter` to confirm the rename

This is a metadata-only operation — no chunk data is copied or moved.
