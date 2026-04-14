# Streaming Guide

## What Is Adaptive Streaming?

When you watch a video on YouTube or listen to a song on Spotify, the app doesn't download the entire file before playing. Instead, it:

1. Asks the server "what quality levels do you have?"
2. Downloads a few seconds of video/audio at a time (called **segments**)
3. Switches between quality levels based on your internet speed

This is called **adaptive bitrate streaming**. The two most common protocols are:

- **HLS** (HTTP Live Streaming) — Apple's protocol. Uses `.m3u8` playlist files. Supported by Safari, iOS, most video players, and most web player libraries.
- **DASH** (Dynamic Adaptive Streaming over HTTP) — An open standard. Uses `.mpd` manifest files. Supported by YouTube, Netflix, and most Android players.

KaidaDB supports both.

## How KaidaDB Handles Streaming

KaidaDB's approach is simple:

1. **You transcode your media externally** (e.g., with FFmpeg) into segments at different quality levels
2. **You store those segments in KaidaDB** as regular media objects, following a naming convention
3. **KaidaDB generates playlists on-the-fly** by looking at what segments you've stored

This means:
- KaidaDB doesn't do any transcoding itself — it stays fast and lightweight
- Segment serving uses the same storage engine, caching, and Range request support as regular media
- You can use any transcoding tool or pipeline you want

### The Key Naming Convention

Streams are organized under the `streams/` prefix (configurable) with this structure:

```
streams/{stream_id}/variants/{variant_id}/init.mp4        # Initialization segment
streams/{stream_id}/variants/{variant_id}/seg-000000.m4s   # Media segment 0
streams/{stream_id}/variants/{variant_id}/seg-000001.m4s   # Media segment 1
...
```

**Example** — A movie with 1080p and 720p video, plus an audio track:

```
streams/inception/variants/1080p/init.mp4
streams/inception/variants/1080p/seg-000000.m4s
streams/inception/variants/1080p/seg-000001.m4s
streams/inception/variants/1080p/seg-000002.m4s
...
streams/inception/variants/720p/init.mp4
streams/inception/variants/720p/seg-000000.m4s
streams/inception/variants/720p/seg-000001.m4s
...
streams/inception/variants/aac-128k/init.mp4
streams/inception/variants/aac-128k/seg-000000.m4s
streams/inception/variants/aac-128k/seg-000001.m4s
...
```

### Metadata on Segments

When storing segments, attach metadata via `X-KaidaDB-Meta-*` headers to tell KaidaDB about each segment's properties.

**On init segments** (one per variant — describes the variant):

| Header | Example | Required |
|--------|---------|----------|
| `X-KaidaDB-Meta-codec` | `avc1.640028` | Yes |
| `X-KaidaDB-Meta-bandwidth` | `5000000` (bits/sec) | Yes |
| `X-KaidaDB-Meta-media-type` | `video`, `audio`, or `subtitle` | Yes |
| `X-KaidaDB-Meta-width` | `1920` | Video only |
| `X-KaidaDB-Meta-height` | `1080` | Video only |
| `X-KaidaDB-Meta-frame-rate` | `30` | Optional |
| `X-KaidaDB-Meta-sample-rate` | `48000` | Audio only |
| `X-KaidaDB-Meta-channels` | `2` | Audio only |
| `X-KaidaDB-Meta-language` | `en` | Optional |

**On media segments** (each segment file):

| Header | Example | Required |
|--------|---------|----------|
| `X-KaidaDB-Meta-segment-index` | `0` (zero-based) | Recommended |
| `X-KaidaDB-Meta-segment-duration` | `4.0` (seconds) | Recommended |

If `segment-index` is omitted, KaidaDB extracts it from the filename (`seg-000042.m4s` → index 42). If `segment-duration` is omitted, it defaults to the configured `target_duration` (4.0 seconds).

## Video Streaming How-To

This walkthrough takes a video file and sets it up for adaptive streaming through KaidaDB.

### Prerequisites

- KaidaDB server running
- FFmpeg installed (`sudo pacman -S ffmpeg` / `sudo apt install ffmpeg` / `brew install ffmpeg`)

### Step 1: Transcode with FFmpeg

Create two video quality levels and an audio-only track, all in fragmented MP4 format:

```bash
INPUT="movie.mp4"
STREAM_ID="my-movie"
SEG_DURATION=4

# 1080p video (no audio)
ffmpeg -i "$INPUT" \
  -c:v libx264 -b:v 5000k -maxrate 5500k -bufsize 10000k \
  -vf "scale=1920:1080" -an \
  -f dash -seg_duration $SEG_DURATION \
  -init_seg_name "init-1080p.mp4" \
  -media_seg_name 'seg-1080p-$Number%06d$.m4s' \
  /tmp/dash-1080p.mpd

# 720p video (no audio)
ffmpeg -i "$INPUT" \
  -c:v libx264 -b:v 2500k -maxrate 2750k -bufsize 5000k \
  -vf "scale=1280:720" -an \
  -f dash -seg_duration $SEG_DURATION \
  -init_seg_name "init-720p.mp4" \
  -media_seg_name 'seg-720p-$Number%06d$.m4s' \
  /tmp/dash-720p.mpd

# Audio only (AAC 128kbps)
ffmpeg -i "$INPUT" \
  -c:a aac -b:a 128k -vn \
  -f dash -seg_duration $SEG_DURATION \
  -init_seg_name "init-aac128.mp4" \
  -media_seg_name 'seg-aac128-$Number%06d$.m4s' \
  /tmp/dash-aac128.mpd
```

Alternatively, produce all variants in one pass using FFmpeg's `fmp4` muxer:

```bash
ffmpeg -i "$INPUT" \
  -map 0:v -map 0:v -map 0:a \
  -c:v:0 libx264 -b:v:0 5000k -vf:v:0 "scale=1920:1080" \
  -c:v:1 libx264 -b:v:1 2500k -vf:v:1 "scale=1280:720" \
  -c:a:0 aac -b:a:0 128k \
  -f dash -seg_duration 4 \
  -adaptation_sets "id=0,streams=v id=1,streams=a" \
  /tmp/output.mpd
```

### Step 2: Upload Init Segments with Metadata

```bash
# 1080p init segment
curl -X PUT \
  -H "Content-Type: video/mp4" \
  -H "X-KaidaDB-Meta-codec: avc1.640028" \
  -H "X-KaidaDB-Meta-bandwidth: 5000000" \
  -H "X-KaidaDB-Meta-media-type: video" \
  -H "X-KaidaDB-Meta-width: 1920" \
  -H "X-KaidaDB-Meta-height: 1080" \
  -H "X-KaidaDB-Meta-frame-rate: 30" \
  -T /tmp/init-1080p.mp4 \
  http://localhost:8080/v1/media/streams/my-movie/variants/1080p/init.mp4

# 720p init segment
curl -X PUT \
  -H "Content-Type: video/mp4" \
  -H "X-KaidaDB-Meta-codec: avc1.64001f" \
  -H "X-KaidaDB-Meta-bandwidth: 2500000" \
  -H "X-KaidaDB-Meta-media-type: video" \
  -H "X-KaidaDB-Meta-width: 1280" \
  -H "X-KaidaDB-Meta-height: 720" \
  -H "X-KaidaDB-Meta-frame-rate: 30" \
  -T /tmp/init-720p.mp4 \
  http://localhost:8080/v1/media/streams/my-movie/variants/720p/init.mp4

# Audio init segment
curl -X PUT \
  -H "Content-Type: audio/mp4" \
  -H "X-KaidaDB-Meta-codec: mp4a.40.2" \
  -H "X-KaidaDB-Meta-bandwidth: 128000" \
  -H "X-KaidaDB-Meta-media-type: audio" \
  -H "X-KaidaDB-Meta-sample-rate: 48000" \
  -H "X-KaidaDB-Meta-channels: 2" \
  -H "X-KaidaDB-Meta-language: en" \
  -T /tmp/init-aac128.mp4 \
  http://localhost:8080/v1/media/streams/my-movie/variants/aac-128k/init.mp4
```

### Step 3: Upload Media Segments

Upload each segment with its index and duration metadata. Here's a script to automate it:

```bash
#!/bin/bash
KAIDADB_URL="http://localhost:8080"
STREAM_ID="my-movie"

upload_segments() {
  local variant="$1"
  local dir="$2"
  local content_type="$3"
  local duration="$4"

  local index=0
  for seg in $(ls "$dir"/seg-*.m4s | sort); do
    local padded=$(printf "%06d" $index)
    curl -X PUT \
      -H "Content-Type: $content_type" \
      -H "X-KaidaDB-Meta-segment-index: $index" \
      -H "X-KaidaDB-Meta-segment-duration: $duration" \
      -T "$seg" \
      "$KAIDADB_URL/v1/media/streams/$STREAM_ID/variants/$variant/seg-${padded}.m4s"
    index=$((index + 1))
  done
  echo "Uploaded $index segments for $variant"
}

upload_segments "1080p"    /tmp/dash-1080p  "video/mp4" "4.0"
upload_segments "720p"     /tmp/dash-720p   "video/mp4" "4.0"
upload_segments "aac-128k" /tmp/dash-aac128 "audio/mp4" "4.0"
```

### Step 4: Play It Back

KaidaDB automatically generates playlists from your stored segments.

**Get the HLS master playlist:**

```bash
curl http://localhost:8080/v1/streams/my-movie/master.m3u8
```

Returns something like:

```
#EXTM3U
#EXT-X-VERSION:7

#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="en",LANGUAGE="en",DEFAULT=YES,URI="/v1/streams/my-movie/variant/aac-128k/playlist.m3u8"

#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,CODECS="avc1.640028",FRAME-RATE=30,AUDIO="audio"
/v1/streams/my-movie/variant/1080p/playlist.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,CODECS="avc1.64001f",FRAME-RATE=30,AUDIO="audio"
/v1/streams/my-movie/variant/720p/playlist.m3u8
```

**Get a variant's media playlist:**

```bash
curl http://localhost:8080/v1/streams/my-movie/variant/1080p/playlist.m3u8
```

**Get a DASH MPD:**

```bash
curl http://localhost:8080/v1/streams/my-movie/manifest.mpd
```

**Play in a browser with hls.js:**

```html
<script src="https://cdn.jsdelivr.net/npm/hls.js@latest"></script>
<video id="video" controls></video>
<script>
  const video = document.getElementById('video');
  if (Hls.isSupported()) {
    const hls = new Hls();
    hls.loadSource('http://localhost:8080/v1/streams/my-movie/master.m3u8');
    hls.attachMedia(video);
  }
</script>
```

**Play with VLC:**

```bash
vlc http://localhost:8080/v1/streams/my-movie/master.m3u8
```

**Play with ffplay:**

```bash
ffplay http://localhost:8080/v1/streams/my-movie/master.m3u8
```

## Music Streaming How-To

Setting up music streaming is simpler — you typically only need one or two audio quality levels, and there's no video to worry about.

### Step 1: Transcode a Song

```bash
INPUT="song.flac"
STREAM_ID="music/artist/album/track-01"
SEG_DURATION=4

# High quality AAC (256kbps)
ffmpeg -i "$INPUT" \
  -c:a aac -b:a 256k \
  -f dash -seg_duration $SEG_DURATION \
  -init_seg_name "init.mp4" \
  -media_seg_name 'seg-$Number%06d$.m4s' \
  /tmp/music-256k.mpd

# Lower quality (128kbps) for slower connections
ffmpeg -i "$INPUT" \
  -c:a aac -b:a 128k \
  -f dash -seg_duration $SEG_DURATION \
  -init_seg_name "init.mp4" \
  -media_seg_name 'seg-$Number%06d$.m4s' \
  /tmp/music-128k.mpd
```

### Step 2: Upload

```bash
# 256k variant - init
curl -X PUT \
  -H "Content-Type: audio/mp4" \
  -H "X-KaidaDB-Meta-codec: mp4a.40.2" \
  -H "X-KaidaDB-Meta-bandwidth: 256000" \
  -H "X-KaidaDB-Meta-media-type: audio" \
  -H "X-KaidaDB-Meta-sample-rate: 44100" \
  -H "X-KaidaDB-Meta-channels: 2" \
  -T /tmp/music-256k/init.mp4 \
  http://localhost:8080/v1/media/streams/music/artist/album/track-01/variants/aac-256k/init.mp4

# 256k variant - segments (same upload_segments script from above)
# ...

# 128k variant
# ...
```

### Step 3: Play

```bash
# HLS
curl http://localhost:8080/v1/streams/music/artist/album/track-01/master.m3u8

# In a web music player
const hls = new Hls();
hls.loadSource('http://localhost:8080/v1/streams/music/artist/album/track-01/master.m3u8');
hls.attachMedia(audioElement);
```

### Batch Processing an Album

Here's a script to transcode and upload an entire album:

```bash
#!/bin/bash
KAIDADB_URL="http://localhost:8080"
ARTIST="pink-floyd"
ALBUM="dark-side-of-the-moon"
QUALITY="aac-256k"

track_num=1
for file in *.flac; do
  TRACK_NAME=$(basename "$file" .flac | tr ' ' '-' | tr '[:upper:]' '[:lower:]')
  PADDED=$(printf "%02d" $track_num)
  STREAM_ID="music/$ARTIST/$ALBUM/$PADDED-$TRACK_NAME"
  VARIANT_PREFIX="streams/$STREAM_ID/variants/$QUALITY"

  echo "Processing: $STREAM_ID"

  # Transcode
  TMPDIR=$(mktemp -d)
  ffmpeg -i "$file" -c:a aac -b:a 256k \
    -f dash -seg_duration 4 \
    -init_seg_name "init.mp4" \
    -media_seg_name 'seg-$Number%06d$.m4s' \
    "$TMPDIR/output.mpd" 2>/dev/null

  # Upload init
  curl -s -X PUT \
    -H "Content-Type: audio/mp4" \
    -H "X-KaidaDB-Meta-codec: mp4a.40.2" \
    -H "X-KaidaDB-Meta-bandwidth: 256000" \
    -H "X-KaidaDB-Meta-media-type: audio" \
    -H "X-KaidaDB-Meta-sample-rate: 44100" \
    -H "X-KaidaDB-Meta-channels: 2" \
    -T "$TMPDIR/init.mp4" \
    "$KAIDADB_URL/v1/media/$VARIANT_PREFIX/init.mp4"

  # Upload segments
  idx=0
  for seg in $(ls "$TMPDIR"/seg-*.m4s 2>/dev/null | sort); do
    padded_idx=$(printf "%06d" $idx)
    curl -s -X PUT \
      -H "Content-Type: audio/mp4" \
      -H "X-KaidaDB-Meta-segment-index: $idx" \
      -H "X-KaidaDB-Meta-segment-duration: 4.0" \
      -T "$seg" \
      "$KAIDADB_URL/v1/media/$VARIANT_PREFIX/seg-${padded_idx}.m4s"
    idx=$((idx + 1))
  done

  rm -rf "$TMPDIR"
  echo "  Uploaded $idx segments"
  track_num=$((track_num + 1))
done

echo "Done. Browse at: $KAIDADB_URL/v1/streams?prefix=music/$ARTIST/$ALBUM/"
```

## Streaming REST Endpoints

| Method | Path | Response |
|--------|------|----------|
| `GET` | `/v1/streams/{stream_id}/master.m3u8` | HLS master playlist (Content-Type: `application/vnd.apple.mpegurl`) |
| `GET` | `/v1/streams/{stream_id}/variant/{variant_id}/playlist.m3u8` | HLS media playlist |
| `GET` | `/v1/streams/{stream_id}/manifest.mpd` | DASH MPD (Content-Type: `application/dash+xml`) |
| `GET` | `/v1/streams?prefix=&limit=&cursor=` | List available streams |
| `DELETE` | `/v1/streams/{stream_id}` | Delete all variants and segments for a stream |

Segment data is served from the existing `GET /v1/media/{key}` endpoint. The playlist URLs point to `/v1/media/streams/...` paths.

All streaming endpoints require the server password for remote access via the `X-Server-Pass` header (see [API Reference](./api-reference.md#authentication)).

## Streaming gRPC RPCs

| RPC | Request | Response |
|-----|---------|----------|
| `GetHlsMasterPlaylist` | `GetPlaylistRequest { stream_id }` | `PlaylistResponse { content_type, body }` |
| `GetHlsMediaPlaylist` | `GetVariantPlaylistRequest { stream_id, variant_id }` | `PlaylistResponse { content_type, body }` |
| `GetDashManifest` | `GetPlaylistRequest { stream_id }` | `PlaylistResponse { content_type, body }` |
| `ListStreams` | `ListStreamsRequest { prefix, limit, cursor }` | `ListStreamsResponse { streams, next_cursor }` |
| `DeleteStream` | `DeleteStreamRequest { stream_id }` | `DeleteStreamResponse { variants_deleted, segments_deleted }` |

## Streaming Configuration

Add a `[streaming]` section to your `config.toml`:

```toml
[streaming]
target_duration = 4.0       # Default segment duration (seconds) for playlist generation
base_url = ""               # Base URL prefix for segment URLs (empty = relative paths)
stream_prefix = "streams/"  # Key prefix where streams are stored
vod_mode = true             # true = include #EXT-X-ENDLIST (finished content)
                            # false = omit it (for live/growing content)
```

| Setting | Default | Description |
|---------|---------|-------------|
| `target_duration` | `4.0` | Used as `#EXT-X-TARGETDURATION` and as fallback when segments don't specify duration |
| `base_url` | `""` | Prepended to all URLs in playlists. Set to your CDN URL if using a CDN in front of KaidaDB |
| `stream_prefix` | `"streams/"` | The key prefix KaidaDB scans for streams. Change if you want a different convention |
| `vod_mode` | `true` | Controls whether HLS playlists include `#EXT-X-ENDLIST`. Set to `false` for live-like behavior where new segments can be added |

Environment variables: `KAIDADB_STREAMING_TARGET_DURATION`, `KAIDADB_STREAMING_BASE_URL`, `KAIDADB_STREAMING_STREAM_PREFIX`, `KAIDADB_STREAMING_VOD_MODE`

## How Playlists Are Generated

Playlists are **never stored** — they're generated on-the-fly every time you request them. Here's what happens:

1. **Master playlist** — KaidaDB lists all keys matching `streams/{stream_id}/variants/*/init.mp4`, reads the metadata from each init segment, and builds the playlist with bandwidth/resolution/codec information
2. **Media playlist** — KaidaDB lists all keys matching `streams/{stream_id}/variants/{variant_id}/seg-*`, extracts duration from metadata, and builds the segment list in order

This means:
- Playlists are always up-to-date (add a segment, it appears in the next playlist request)
- No consistency issues between stored playlists and actual segments
- No extra storage for playlist files

The lookup is fast because the index is an in-memory BTreeMap — prefix scans are efficient range queries.

## Tips

- **Segment duration** — 4 seconds is a good default. Shorter segments (2s) give faster quality switching but more overhead. Longer segments (10s) reduce overhead but make quality switches slower.
- **Codec strings** — Use the proper codec string for your encoding. Common values: `avc1.640028` (H.264 High 4.0), `avc1.64001f` (H.264 High 3.1), `mp4a.40.2` (AAC-LC), `hev1.1.6.L93.B0` (H.265/HEVC).
- **Zero-padded segment numbers** — Always use 6-digit zero-padding (`seg-000042.m4s`, not `seg-42.m4s`). This ensures lexicographic sorting matches playback order.
- **fMP4 format** — Use fragmented MP4 (`.m4s`) segments, not MPEG-TS (`.ts`). fMP4 supports both HLS and DASH from the same segments, saving storage.
- **Deduplication bonus** — If multiple variants share identical audio, the audio chunks are deduplicated automatically by KaidaDB's content-addressed storage.
