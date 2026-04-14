# When to Use KaidaDB

## What KaidaDB Is

KaidaDB is a **self-hosted media storage and streaming engine**. It stores large binary files (video, audio, images) on your own hardware, streams them efficiently, and handles HLS/DASH adaptive bitrate delivery. It's a single binary with no external dependencies — no cloud, no separate database, no message queue.

It is **not** a general-purpose database. It's a specialized tool for one job: storing and serving media files.

## Strengths

### You Own Everything

KaidaDB runs on your hardware. Your media stays on your disks. There are no API keys, no monthly bills, no bandwidth charges, no terms of service, and no vendor that can shut down or change pricing. If the power is on, KaidaDB is available.

### Single Binary, Zero Dependencies

No PostgreSQL, no Redis, no S3, no Kafka, no Docker required. Install one binary, point it at a directory, and it works. This matters when you're self-hosting — every dependency is something else to configure, update, and debug at 2 AM.

### Built for Large Files

Most databases choke on large binary data. They'll store a 4 GB video in a BLOB column, but reading it back means loading the entire thing into memory. KaidaDB stores files as chains of fixed-size chunks and streams them back piece by piece. It never loads an entire file into memory, even during upload or download.

### Content Deduplication

Identical chunks across different files are stored only once. If you have 100 TV episodes with the same 30-second intro, those intro chunks exist once on disk. This is automatic — you don't need to configure or think about it.

### Streaming Built In

KaidaDB handles HLS and DASH playlist generation natively. Combined with an external transcoder (FFmpeg), you get a complete streaming server without bolting together Nginx, a manifest generator, and a CDN origin.

### HTTP Range Requests

Media players and browsers use Range requests to seek within files. KaidaDB supports this natively with O(1) chunk lookup — seeking to any point in a 50 GB file is instant, not proportional to file size.

### Flexible Key System

Keys are plain strings with `/` for hierarchy. There's no rigid schema, no migrations, no predefined structure. You organize your media however makes sense for your project. The same KaidaDB instance can serve a video library, a music collection, and a security camera archive.

### Cache Awareness

An LRU cache keeps popular media chunks in memory. For a music server where 20% of tracks get 80% of plays, this means most playback hits RAM, not disk. The cache manages itself — no manual warming or invalidation needed.

## Limitations

### No Query Language

KaidaDB supports exactly two ways to find media: by exact key, or by key prefix. You cannot query "all videos longer than 30 minutes" or "all songs by artist X" from KaidaDB alone. If you need search, filter by metadata, or relational queries, you need a separate database (Postgres, SQLite, Elasticsearch) to index the metadata.

### No Transcoding

KaidaDB stores and serves media — it does not convert between formats. To use HLS/DASH streaming, you must transcode your files externally (FFmpeg) and upload the segments. This is by design (transcoding is CPU-heavy and would degrade storage performance), but it means more setup compared to an all-in-one media server.

### No Multi-User Access Control

KaidaDB has a server-wide password that protects remote access — local connections work without auth, remote connections require the auto-generated password. However, it has no user accounts, roles, or per-key permissions. Anyone with the server password has full read/write/delete access to everything. For fine-grained access control, put a reverse proxy or API gateway in front of it.

### No Encryption at Rest

Data is stored as plain files on disk. If you need encryption, use filesystem-level encryption (LUKS, dm-crypt) or encrypted storage volumes.

### No Replication (Yet)

KaidaDB is currently single-node. If the disk fails, data is lost unless you have backups. Distributed clustering with replication is on the roadmap (Phase 3) but not yet implemented.

### Index Must Fit in RAM

The index (mapping keys to chunks) lives entirely in memory. For most media workloads this is fine — 1 million media objects use a few hundred MB of RAM. But if you plan to store tens of millions of small files, the index memory could become significant.

### No Live Streaming Ingest

KaidaDB can serve HLS/DASH playlists for pre-recorded content. It does not accept RTMP or SRT ingest for live streaming. You could build a live pipeline by continuously uploading segments from an external encoder, but KaidaDB doesn't handle the ingest side.

## When to Use KaidaDB

**Use KaidaDB when:**

- You're building a **self-hosted media application** (video library, music player, podcast archive, photo backup)
- You want a **personal streaming server** that handles HLS/DASH without cloud services
- You need efficient storage for **large binary files** with deduplication
- You want a **simple, dependency-free media backend** for a web or mobile app
- You're building a **security camera archive** where media is written once and streamed on demand
- You want to **own your media infrastructure** without recurring cloud costs
- Your project needs a **media storage layer** that handles chunking, caching, and streaming so you don't have to

**Real-world project examples:**
- A Jellyfin/Plex alternative where KaidaDB handles storage and streaming while your app handles the UI and metadata
- A podcast hosting platform where episodes are stored in KaidaDB and served via HLS
- A family photo/video archive accessible from any device on your home network
- A music server that streams FLAC or AAC to a web player
- A NVR (network video recorder) that writes camera feeds as segments

## When NOT to Use KaidaDB

**Don't use KaidaDB when:**

- You need a **general-purpose database** — use PostgreSQL, MySQL, or SQLite
- You need **full-text search** or complex queries — use Elasticsearch, Meilisearch, or PostgreSQL
- You're storing **small structured data** (user profiles, settings, logs) — use a regular database
- You need **real-time collaboration** (documents, shared state) — use a database with transactions
- You need **live streaming ingest** (RTMP/SRT) — use OBS + a dedicated ingest server
- You need **DRM/content protection** — use a commercial streaming platform
- You need **multi-region, globally distributed** storage — use S3 + CloudFront or similar
- You need to query media **by attributes** (find all 4K videos, sort by date) — KaidaDB only supports key-based and prefix-based lookup. Pair it with a metadata database for attribute queries.

## How KaidaDB Compares

KaidaDB is not a replacement for:

| Tool | What It Does | When to Choose It Over KaidaDB |
|------|-------------|-------------------------------|
| **PostgreSQL** | Relational database | You need SQL queries, transactions, relational data |
| **S3 / MinIO** | Object storage | You need S3 API compatibility, multi-region, or IAM |
| **Jellyfin / Plex** | Full media server with UI | You want an out-of-the-box media player with metadata scraping |
| **Nginx** | Web server with static file serving | Your files are already on disk and you just need HTTP serving |
| **SeaweedFS** | Distributed file system | You need distributed storage across many nodes today |

KaidaDB **complements** these tools:

- Use KaidaDB as the storage backend for a Jellyfin-like app you build yourself
- Use KaidaDB as the origin server behind an Nginx/Caddy reverse proxy
- Use KaidaDB alongside PostgreSQL — Postgres handles metadata/search, KaidaDB handles media bytes

## The KaidaDB Philosophy

KaidaDB is built on a few core beliefs:

1. **Self-hosting should be easy.** One binary, one config file, one data directory. No infrastructure team required.
2. **Media storage is a specialized problem.** General-purpose databases are bad at storing and streaming large binary files. A purpose-built tool does it better.
3. **Flexibility over features.** KaidaDB gives you a solid storage and streaming foundation. What you build on top of it is up to you.
4. **No cloud required.** You should be able to run your media infrastructure on a Raspberry Pi in your closet if you want to.
