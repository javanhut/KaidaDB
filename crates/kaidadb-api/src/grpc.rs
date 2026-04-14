use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use kaidadb_cache::ChunkCache;
use kaidadb_common::config::StreamingConfig;
use kaidadb_common::server_key;
use kaidadb_storage::StorageEngine;

use crate::proto::kaida_db_server::KaidaDb;
use crate::proto::*;
use crate::streaming;

pub struct KaidaDbGrpc {
    engine: Arc<StorageEngine>,
    cache: Arc<ChunkCache>,
    streaming: StreamingConfig,
    server_key_hash: Arc<String>,
}

impl KaidaDbGrpc {
    pub fn new(
        engine: Arc<StorageEngine>,
        cache: Arc<ChunkCache>,
        streaming: StreamingConfig,
        server_key_hash: Arc<String>,
    ) -> Self {
        Self {
            engine,
            cache,
            streaming,
            server_key_hash,
        }
    }

    fn check_auth<T>(&self, req: &Request<T>) -> Result<(), Status> {
        // Allow local connections without auth
        if let Some(addr) = req.remote_addr() {
            if addr.ip().is_loopback() {
                return Ok(());
            }
        } else {
            // No remote addr available (e.g., UDS) — treat as local
            return Ok(());
        }

        // Remote connection: require server password
        match req.metadata().get("x-server-pass") {
            Some(value) => {
                let provided = value.to_str().map_err(|_| {
                    Status::unauthenticated("invalid server password encoding")
                })?;
                if server_key::verify_key(provided, &self.server_key_hash) {
                    Ok(())
                } else {
                    Err(Status::unauthenticated("invalid server password"))
                }
            }
            None => Err(Status::unauthenticated(
                "server password required for remote access (use --server-pass)",
            )),
        }
    }
}

#[tonic::async_trait]
impl KaidaDb for KaidaDbGrpc {
    async fn store_media(
        &self,
        request: Request<Streaming<StoreMediaRequest>>,
    ) -> Result<Response<StoreMediaResponse>, Status> {
        self.check_auth(&request)?;
        let mut stream = request.into_inner();

        let mut key = String::new();
        let mut content_type = String::new();
        let mut metadata = std::collections::HashMap::new();
        let mut data = Vec::new();

        while let Some(msg) = stream
            .message()
            .await
            .map_err(|e| Status::internal(e.to_string()))?
        {
            match msg.request {
                Some(store_media_request::Request::Header(header)) => {
                    key = header.key;
                    content_type = header.content_type;
                    metadata = header.metadata;
                }
                Some(store_media_request::Request::ChunkData(chunk)) => {
                    data.extend_from_slice(&chunk);
                }
                None => {}
            }
        }

        if key.is_empty() {
            return Err(Status::invalid_argument("key is required"));
        }

        let manifest = self
            .engine
            .store_with_metadata(&key, &data, &content_type, metadata)
            .map_err(|e| Status::internal(e.to_string()))?;

        let chunk_count = manifest.chunk_count();
        Ok(Response::new(StoreMediaResponse {
            key: manifest.key,
            total_size: manifest.total_size,
            chunk_count,
            checksum: manifest.checksum,
        }))
    }

    type StreamMediaStream = ReceiverStream<Result<MediaChunk, Status>>;

    async fn stream_media(
        &self,
        request: Request<StreamMediaRequest>,
    ) -> Result<Response<Self::StreamMediaStream>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();

        let manifest = self
            .engine
            .get_manifest(&req.key)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found(format!("key not found: {}", req.key)))?;

        let offset = req.offset;
        let length = req.length;
        let total_size = manifest.total_size;

        let end = if length == 0 {
            total_size
        } else {
            (offset + length).min(total_size)
        };

        if offset >= total_size {
            let (tx, rx) = mpsc::channel(1);
            drop(tx);
            return Ok(Response::new(ReceiverStream::new(rx)));
        }

        let chunk_size = manifest.chunk_size as u64;
        let start_chunk_idx = (offset / chunk_size) as usize;
        let end_chunk_idx = ((end - 1) / chunk_size) as usize;

        let (tx, rx) = mpsc::channel(4);
        let engine = self.engine.clone();
        let cache = self.cache.clone();
        let chunk_ids = manifest.chunks.clone();

        tokio::spawn(async move {
            let mut sequence = 0u32;
            let mut current_offset = offset;

            for idx in start_chunk_idx..=end_chunk_idx.min(chunk_ids.len() - 1) {
                let chunk_id = &chunk_ids[idx];

                // Try cache first
                let chunk_data = if let Some(cached) = cache.get(chunk_id) {
                    cached
                } else {
                    // Read from storage
                    match engine.read_chunk(chunk_id) {
                        Ok(data) => {
                            cache.insert(chunk_id.clone(), data.clone());
                            data
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Err(Status::internal(e.to_string())))
                                .await;
                            return;
                        }
                    }
                };

                let chunk_start = idx as u64 * chunk_size;
                let slice_start = if idx == start_chunk_idx {
                    (offset - chunk_start) as usize
                } else {
                    0
                };
                let slice_end = if idx == end_chunk_idx {
                    (end - chunk_start) as usize
                } else {
                    chunk_data.len()
                };
                let slice_end = slice_end.min(chunk_data.len());
                let slice = &chunk_data[slice_start..slice_end];

                let is_last = idx == end_chunk_idx.min(chunk_ids.len() - 1);

                let chunk = MediaChunk {
                    sequence,
                    data: slice.to_vec(),
                    offset: current_offset,
                    is_last,
                };

                current_offset += slice.len() as u64;
                sequence += 1;

                if tx.send(Ok(chunk)).await.is_err() {
                    return;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn get_media_meta(
        &self,
        request: Request<GetMediaMetaRequest>,
    ) -> Result<Response<MediaMetadata>, Status> {
        self.check_auth(&request)?;
        let key = &request.into_inner().key;
        let manifest = self
            .engine
            .get_manifest(key)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found(format!("key not found: {key}")))?;

        Ok(Response::new(manifest_to_metadata(&manifest)))
    }

    async fn delete_media(
        &self,
        request: Request<DeleteMediaRequest>,
    ) -> Result<Response<DeleteMediaResponse>, Status> {
        self.check_auth(&request)?;
        let key = &request.into_inner().key;

        // Invalidate cached chunks
        if let Ok(Some(manifest)) = self.engine.get_manifest(key) {
            for chunk_id in &manifest.chunks {
                self.cache.invalidate(chunk_id);
            }
        }

        let deleted = self
            .engine
            .delete(key)
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(DeleteMediaResponse { deleted }))
    }

    async fn rename_media(
        &self,
        request: Request<RenameMediaRequest>,
    ) -> Result<Response<RenameMediaResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();

        if req.from_key.is_empty() || req.to_key.is_empty() {
            return Err(Status::invalid_argument("from_key and to_key are required"));
        }

        let manifest = self
            .engine
            .rename(&req.from_key, &req.to_key)
            .map_err(|e| match e {
                kaidadb_common::KaidaDbError::NotFound(_) => Status::not_found(e.to_string()),
                kaidadb_common::KaidaDbError::AlreadyExists(_) => {
                    Status::already_exists(e.to_string())
                }
                kaidadb_common::KaidaDbError::InvalidKey(_) => {
                    Status::invalid_argument(e.to_string())
                }
                _ => Status::internal(e.to_string()),
            })?;

        Ok(Response::new(RenameMediaResponse {
            key: manifest.key,
            total_size: manifest.total_size,
            content_type: manifest.content_type,
        }))
    }

    async fn list_media(
        &self,
        request: Request<ListMediaRequest>,
    ) -> Result<Response<ListMediaResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let limit = if req.limit == 0 { 100 } else { req.limit as usize };

        let (manifests, next_cursor) = self
            .engine
            .list(&req.prefix, limit, &req.cursor)
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ListMediaResponse {
            items: manifests.iter().map(manifest_to_metadata).collect(),
            next_cursor: next_cursor.unwrap_or_default(),
        }))
    }

    async fn health_check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        self.check_auth(&request)?;
        Ok(Response::new(HealthCheckResponse {
            status: "ok".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            uptime_seconds: 0, // TODO: track uptime
        }))
    }

    async fn get_hls_master_playlist(
        &self,
        request: Request<GetPlaylistRequest>,
    ) -> Result<Response<PlaylistResponse>, Status> {
        self.check_auth(&request)?;
        let stream_id = &request.into_inner().stream_id;
        let variants = streaming::discover_variants(&self.engine, &self.streaming, stream_id)
            .map_err(|e| Status::internal(e))?;

        if variants.is_empty() {
            return Err(Status::not_found(format!("stream not found: {}", stream_id)));
        }

        let body = streaming::generate_hls_master(&variants, stream_id, &self.streaming);
        Ok(Response::new(PlaylistResponse {
            content_type: "application/vnd.apple.mpegurl".into(),
            body,
        }))
    }

    async fn get_hls_media_playlist(
        &self,
        request: Request<GetVariantPlaylistRequest>,
    ) -> Result<Response<PlaylistResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let init_key = format!(
            "{}{}variants/{}/init.mp4",
            self.streaming.stream_prefix, req.stream_id, req.variant_id
        );

        self.engine
            .get_manifest(&init_key)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found(format!("variant not found: {}", req.variant_id)))?;

        let segments = streaming::discover_segments(
            &self.engine,
            &self.streaming,
            &req.stream_id,
            &req.variant_id,
        )
        .map_err(|e| Status::internal(e))?;

        let body = streaming::generate_hls_media(&segments, &init_key, &self.streaming);
        Ok(Response::new(PlaylistResponse {
            content_type: "application/vnd.apple.mpegurl".into(),
            body,
        }))
    }

    async fn get_dash_manifest(
        &self,
        request: Request<GetPlaylistRequest>,
    ) -> Result<Response<PlaylistResponse>, Status> {
        self.check_auth(&request)?;
        let stream_id = &request.into_inner().stream_id;
        let variants = streaming::discover_variants(&self.engine, &self.streaming, stream_id)
            .map_err(|e| Status::internal(e))?;

        if variants.is_empty() {
            return Err(Status::not_found(format!("stream not found: {}", stream_id)));
        }

        let mut segments_by_variant = Vec::new();
        for variant in &variants {
            let segs = streaming::discover_segments(
                &self.engine,
                &self.streaming,
                stream_id,
                &variant.variant_id,
            )
            .map_err(|e| Status::internal(e))?;
            segments_by_variant.push((variant.variant_id.clone(), segs));
        }

        let body =
            streaming::generate_dash_mpd(&variants, &segments_by_variant, &self.streaming);
        Ok(Response::new(PlaylistResponse {
            content_type: "application/dash+xml".into(),
            body,
        }))
    }

    async fn list_streams(
        &self,
        request: Request<ListStreamsRequest>,
    ) -> Result<Response<ListStreamsResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let limit = if req.limit == 0 { 100 } else { req.limit as usize };

        let (items, next_cursor) =
            streaming::list_streams(&self.engine, &self.streaming, &req.prefix, limit, &req.cursor)
                .map_err(|e| Status::internal(e))?;

        let streams = items
            .into_iter()
            .map(|item| StreamInfo {
                stream_id: item.stream_id,
                variants: Vec::new(),
                created_at: 0,
            })
            .collect();

        Ok(Response::new(ListStreamsResponse {
            streams,
            next_cursor: next_cursor.unwrap_or_default(),
        }))
    }

    async fn delete_stream(
        &self,
        request: Request<DeleteStreamRequest>,
    ) -> Result<Response<DeleteStreamResponse>, Status> {
        self.check_auth(&request)?;
        let stream_id = &request.into_inner().stream_id;

        // Invalidate cache
        let prefix = format!("{}{}/", self.streaming.stream_prefix, stream_id);
        if let Ok((manifests, _)) = self.engine.list(&prefix, 100000, "") {
            for manifest in &manifests {
                for chunk_id in &manifest.chunks {
                    self.cache.invalidate(chunk_id);
                }
            }
        }

        let (variants_deleted, segments_deleted) =
            streaming::delete_stream(&self.engine, &self.streaming, stream_id)
                .map_err(|e| Status::internal(e))?;

        Ok(Response::new(DeleteStreamResponse {
            variants_deleted,
            segments_deleted,
        }))
    }
}

fn manifest_to_metadata(m: &kaidadb_common::MediaManifest) -> MediaMetadata {
    MediaMetadata {
        key: m.key.clone(),
        total_size: m.total_size,
        chunk_count: m.chunk_count(),
        content_type: m.content_type.clone(),
        checksum: m.checksum.clone(),
        metadata: m.metadata.clone(),
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}
