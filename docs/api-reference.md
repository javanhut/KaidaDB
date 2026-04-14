# API Reference

KaidaDB exposes two APIs — REST and gRPC — that share the same storage engine. Both can be used simultaneously. Use whichever fits your application.

## Authentication

All endpoints on both APIs are protected by a server password for remote access. See the [Getting Started guide](./getting-started.md#server-password-remote-access-security) for setup details.

- **Local requests** (from 127.0.0.1 / ::1) pass through without authentication
- **Remote requests** must include the server password:
  - **REST**: `X-Server-Pass` header
  - **gRPC**: `x-server-pass` metadata key

Without the correct password, remote requests receive `401 Unauthorized` (REST) or `UNAUTHENTICATED` (gRPC).

## REST API

The REST API listens on port 8080 by default. All responses use JSON unless otherwise noted.

### Media Endpoints

#### `PUT /v1/media/{key}` — Store Media

Store a file under the given key. The key can contain slashes for hierarchical organization.

**Request:**
- Body: raw file bytes
- `Content-Type` header: MIME type of the media (e.g., `video/mp4`, `audio/flac`)
- `X-KaidaDB-Meta-*` headers: custom metadata key-value pairs

**Response:** `201 Created`

```json
{
  "key": "movies/inception",
  "total_size": 1234567890,
  "chunk_count": 589,
  "checksum": "a1b2c3d4e5f6..."
}
```

**Example:**

```bash
curl -X PUT \
  -H "Content-Type: video/mp4" \
  -H "X-KaidaDB-Meta-resolution: 4k" \
  -H "X-KaidaDB-Meta-language: en" \
  -T movie.mp4 \
  http://localhost:8080/v1/media/movies/inception
```

If the key already exists, the old media is replaced (overwritten).

---

#### `GET /v1/media/{key}` — Retrieve Media

Download the media stored under the given key. Supports HTTP Range requests for partial downloads and seeking.

**Response:** `200 OK` (full file) or `206 Partial Content` (range request)

**Headers returned:**
- `Content-Type`: the stored MIME type
- `Content-Length`: size in bytes
- `Accept-Ranges: bytes`
- `Content-Range: bytes start-end/total` (only for 206 responses)

**Examples:**

```bash
# Full download
curl http://localhost:8080/v1/media/movies/inception -o movie.mp4

# First 1 MiB
curl -H "Range: bytes=0-1048575" http://localhost:8080/v1/media/movies/inception

# Bytes 100-199
curl -H "Range: bytes=100-199" http://localhost:8080/v1/media/movies/inception

# Last 500 bytes
curl -H "Range: bytes=-500" http://localhost:8080/v1/media/movies/inception

# From byte 1000000 to end
curl -H "Range: bytes=1000000-" http://localhost:8080/v1/media/movies/inception
```

**Error:** `404 Not Found` if the key doesn't exist.

---

#### `HEAD /v1/media/{key}` — Get Metadata (Headers Only)

Returns metadata as response headers without downloading the file body. Useful for checking if a file exists and its properties.

**Response:** `200 OK` with empty body

**Headers returned:**
- `Content-Type`: stored MIME type
- `Content-Length`: size in bytes
- `Accept-Ranges: bytes`
- `X-KaidaDB-Checksum`: SHA-256 hash of the full file
- `X-KaidaDB-Chunk-Count`: number of chunks
- `X-KaidaDB-Meta-*`: any custom metadata that was stored

**Example:**

```bash
curl -I http://localhost:8080/v1/media/movies/inception
```

**Error:** `404 Not Found` if the key doesn't exist.

---

#### `DELETE /v1/media/{key}` — Delete Media

Remove a media object. Chunks that are no longer referenced by any other media are also deleted from disk.

**Response:** `204 No Content` on success, `404 Not Found` if the key doesn't exist.

```bash
curl -X DELETE http://localhost:8080/v1/media/movies/inception
```

---

#### `GET /v1/media?prefix=&limit=&cursor=` — List Media

List media keys with optional prefix filtering and cursor-based pagination.

**Query parameters:**

| Parameter | Default | Description |
|-----------|---------|-------------|
| `prefix` | `""` | Filter to keys starting with this prefix |
| `limit` | `100` | Maximum number of results to return |
| `cursor` | `""` | Pagination cursor from a previous response |

**Response:** `200 OK`

```json
{
  "items": [
    {
      "key": "movies/inception",
      "total_size": 1234567890,
      "chunk_count": 589,
      "content_type": "video/mp4",
      "checksum": "a1b2c3d4...",
      "created_at": 1710000000
    }
  ],
  "next_cursor": "movies/the-matrix"
}
```

If `next_cursor` is `null`, there are no more results. Otherwise, pass it as the `cursor` parameter in the next request to get the next page.

**Examples:**

```bash
# List everything
curl http://localhost:8080/v1/media

# List movies
curl "http://localhost:8080/v1/media?prefix=movies/"

# Paginate
curl "http://localhost:8080/v1/media?prefix=tv/&limit=20"
curl "http://localhost:8080/v1/media?prefix=tv/&limit=20&cursor=tv/the-wire/s01/e03"
```

---

#### `GET /v1/meta/{key}` — Get Metadata (JSON)

Returns full metadata as a JSON body, including custom metadata and timestamps.

**Response:** `200 OK`

```json
{
  "key": "movies/inception",
  "total_size": 1234567890,
  "chunk_count": 589,
  "content_type": "video/mp4",
  "checksum": "a1b2c3d4e5f6...",
  "metadata": {
    "resolution": "4k",
    "language": "en"
  },
  "created_at": 1710000000,
  "updated_at": 1710000000
}
```

**Error:** `404 Not Found` if the key doesn't exist.

---

#### `POST /v1/media/rename` — Rename Media

Move a media object from one key to another. The underlying chunk data is not copied — only the manifest key is changed.

**Request body (JSON):**

```json
{
  "from_key": "movies/old-name",
  "to_key": "movies/new-name"
}
```

**Response:** `200 OK`

```json
{
  "key": "movies/new-name",
  "total_size": 1234567890,
  "content_type": "video/mp4"
}
```

**Errors:**
- `404 Not Found` — `from_key` doesn't exist
- `409 Conflict` — `to_key` already exists

---

#### `GET /v1/health` — Health Check

**Response:** `200 OK`

```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

### Streaming Endpoints

See the [Streaming Guide](./streaming-guide.md) for full details on setting up streaming.

#### `GET /v1/streams/{stream_id}/master.m3u8` — HLS Master Playlist

Returns an HLS master playlist listing all available quality variants.

**Response:** `200 OK` with `Content-Type: application/vnd.apple.mpegurl`

**Error:** `404 Not Found` if no variants exist for the stream.

---

#### `GET /v1/streams/{stream_id}/variant/{variant_id}/playlist.m3u8` — HLS Media Playlist

Returns an HLS media playlist listing all segments for a specific variant.

**Response:** `200 OK` with `Content-Type: application/vnd.apple.mpegurl`

**Error:** `404 Not Found` if the variant doesn't exist.

---

#### `GET /v1/streams/{stream_id}/manifest.mpd` — DASH MPD

Returns a DASH MPD manifest for the stream.

**Response:** `200 OK` with `Content-Type: application/dash+xml`

**Error:** `404 Not Found` if no variants exist for the stream.

---

#### `GET /v1/streams?prefix=&limit=&cursor=` — List Streams

List available streams with optional prefix filtering.

**Response:** `200 OK`

```json
{
  "items": [
    {
      "stream_id": "my-movie",
      "variant_count": 3
    }
  ],
  "next_cursor": null
}
```

---

#### `DELETE /v1/streams/{stream_id}` — Delete Stream

Delete all variants and segments for a stream.

**Response:** `200 OK`

```json
{
  "variants_deleted": 3,
  "segments_deleted": 450
}
```

### Custom Metadata

You can attach arbitrary key-value metadata to any media object using `X-KaidaDB-Meta-*` headers on PUT requests:

```bash
curl -X PUT \
  -H "Content-Type: video/mp4" \
  -H "X-KaidaDB-Meta-director: Christopher Nolan" \
  -H "X-KaidaDB-Meta-year: 2010" \
  -H "X-KaidaDB-Meta-rating: PG-13" \
  -T movie.mp4 \
  http://localhost:8080/v1/media/movies/inception
```

Metadata is returned:
- As `X-KaidaDB-Meta-*` response headers on `HEAD` requests
- In the `metadata` object on `GET /v1/meta/{key}` responses

Metadata keys are stored lowercase (HTTP headers are case-insensitive, so `X-KaidaDB-Meta-Director` becomes `director`).

### Error Responses

| Status Code | Meaning |
|-------------|---------|
| `200` | Success |
| `201` | Created (PUT) |
| `204` | No Content (DELETE success) |
| `206` | Partial Content (Range request) |
| `400` | Bad Request (invalid key, malformed request) |
| `401` | Unauthorized (missing or invalid server password for remote access) |
| `404` | Not Found |
| `405` | Method Not Allowed |
| `409` | Conflict (rename target already exists) |
| `500` | Internal Server Error |

### CORS

The REST API returns permissive CORS headers, allowing requests from any origin. This means you can call it directly from browser JavaScript without a proxy.

---

## gRPC API

The gRPC API listens on port 50051 by default. It's defined in `proto/kaidadb.proto`.

### Service Definition

```protobuf
service KaidaDB {
    // Upload media (client-streaming)
    rpc StoreMedia(stream StoreMediaRequest) returns (StoreMediaResponse);

    // Download media (server-streaming)
    rpc StreamMedia(StreamMediaRequest) returns (stream MediaChunk);

    // Get metadata
    rpc GetMediaMeta(GetMediaMetaRequest) returns (MediaMetadata);

    // Delete media
    rpc DeleteMedia(DeleteMediaRequest) returns (DeleteMediaResponse);

    // List media with pagination
    rpc ListMedia(ListMediaRequest) returns (ListMediaResponse);

    // Rename/move media
    rpc RenameMedia(RenameMediaRequest) returns (RenameMediaResponse);

    // Health check
    rpc HealthCheck(HealthCheckRequest) returns (HealthCheckResponse);

    // Streaming playlists
    rpc GetHlsMasterPlaylist(GetPlaylistRequest) returns (PlaylistResponse);
    rpc GetHlsMediaPlaylist(GetVariantPlaylistRequest) returns (PlaylistResponse);
    rpc GetDashManifest(GetPlaylistRequest) returns (PlaylistResponse);
    rpc ListStreams(ListStreamsRequest) returns (ListStreamsResponse);
    rpc DeleteStream(DeleteStreamRequest) returns (DeleteStreamResponse);
}
```

### StoreMedia (Client-Streaming Upload)

Send a header message first with the key, content type, and metadata, followed by one or more chunk_data messages:

```protobuf
message StoreMediaRequest {
    oneof request {
        StoreMediaHeader header = 1;  // Send first
        bytes chunk_data = 2;         // Then send data in chunks
    }
}

message StoreMediaHeader {
    string key = 1;
    string content_type = 2;
    map<string, string> metadata = 3;
}

message StoreMediaResponse {
    string key = 1;
    uint64 total_size = 2;
    uint32 chunk_count = 3;
    string checksum = 4;
}
```

### StreamMedia (Server-Streaming Download)

Request a key with optional byte range:

```protobuf
message StreamMediaRequest {
    string key = 1;
    uint64 offset = 2;   // Byte offset to start from (0 = beginning)
    uint64 length = 3;   // Bytes to read (0 = read to end)
}

message MediaChunk {
    uint32 sequence = 1;  // Ordered chunk number (0, 1, 2, ...)
    bytes data = 2;       // Chunk payload
    uint64 offset = 3;    // Absolute byte offset in the file
    bool is_last = 4;     // True for the final chunk
}
```

### MediaMetadata

Returned by `GetMediaMeta` and `ListMedia`:

```protobuf
message MediaMetadata {
    string key = 1;
    uint64 total_size = 2;
    uint32 chunk_count = 3;
    string content_type = 4;
    string checksum = 5;
    map<string, string> metadata = 6;
    int64 created_at = 7;
    int64 updated_at = 8;
}
```

### ListMedia (Pagination)

```protobuf
message ListMediaRequest {
    string prefix = 1;   // Filter by key prefix
    uint32 limit = 2;    // Max results (default: 100)
    string cursor = 3;   // Cursor from previous response
}

message ListMediaResponse {
    repeated MediaMetadata items = 1;
    string next_cursor = 2;  // Empty string = no more results
}
```

### gRPC Error Codes

| gRPC Status | When |
|-------------|------|
| `UNAUTHENTICATED` | Missing or invalid server password for remote access |
| `NOT_FOUND` | Key doesn't exist |
| `ALREADY_EXISTS` | Rename target already exists |
| `INVALID_ARGUMENT` | Missing required fields, invalid key |
| `INTERNAL` | Storage or I/O error |
