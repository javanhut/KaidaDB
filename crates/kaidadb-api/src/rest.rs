use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use percent_encoding::percent_decode_str;
use tower_http::cors::CorsLayer;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use kaidadb_cache::ChunkCache;
use kaidadb_storage::StorageEngine;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<StorageEngine>,
    pub cache: Arc<ChunkCache>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/media", get(list_media))
        .route("/v1/health", get(health))
        .fallback(media_fallback)
        .with_state(state)
        .layer(CorsLayer::permissive())
}

/// Wildcard captures include a leading `/`; strip it so keys stay consistent.
fn normalize_key(raw: String) -> String {
    match raw.strip_prefix('/') {
        Some(s) => s.to_string(),
        None => raw,
    }
}

async fn put_media(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let key = normalize_key(raw_key);
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    // Extract custom metadata from X-KaidaDB-Meta-* headers
    let mut metadata = std::collections::HashMap::new();
    for (name, value) in &headers {
        if let Some(meta_key) = name.as_str().strip_prefix("x-kaidadb-meta-") {
            if let Ok(v) = value.to_str() {
                metadata.insert(meta_key.to_string(), v.to_string());
            }
        }
    }

    match state
        .engine
        .store_with_metadata(&key, &body, &content_type, metadata)
    {
        Ok(manifest) => {
            let resp = serde_json::json!({
                "key": manifest.key,
                "total_size": manifest.total_size,
                "chunk_count": manifest.chunk_count(),
                "checksum": manifest.checksum,
            });
            (StatusCode::CREATED, axum::Json(resp)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_media(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let key = normalize_key(raw_key);
    let manifest = match state.engine.get_manifest(&key) {
        Ok(Some(m)) => m,
        Ok(None) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let total_size = manifest.total_size;
    let content_type = manifest.content_type.clone();

    // Parse Range header
    let (offset, length, is_range) = parse_range_header(&headers, total_size);

    let status = if is_range {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };

    let end = if length == 0 {
        total_size
    } else {
        (offset + length).min(total_size)
    };

    // Stream the response body
    let engine = state.engine.clone();
    let cache = state.cache.clone();
    let chunk_size = manifest.chunk_size as u64;
    let chunks = manifest.chunks.clone();

    let start_idx = (offset / chunk_size) as usize;
    let end_idx = if end == 0 {
        0
    } else {
        ((end - 1) / chunk_size) as usize
    };

    // Pre-warm first chunks into LRU cache to eliminate disk I/O latency
    // for the critical initial bytes (important when ffmpeg reads the header).
    let prefetch_end = (end_idx + 1).min(start_idx + 3).min(chunks.len());
    for idx in start_idx..prefetch_end {
        let chunk_id = &chunks[idx];
        if cache.get(chunk_id).is_none() {
            if let Ok(data) = engine.read_chunk(chunk_id) {
                cache.insert(chunk_id.clone(), data);
            }
        }
    }

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);

    tokio::spawn(async move {
        for idx in start_idx..=end_idx.min(chunks.len().saturating_sub(1)) {
            let chunk_id = &chunks[idx];

            let chunk_data = if let Some(cached) = cache.get(chunk_id) {
                cached
            } else {
                match engine.read_chunk(chunk_id) {
                    Ok(data) => {
                        cache.insert(chunk_id.clone(), data.clone());
                        data
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
                            .await;
                        return;
                    }
                }
            };

            let chunk_start = idx as u64 * chunk_size;
            let slice_start = if idx == start_idx {
                (offset - chunk_start) as usize
            } else {
                0
            };
            let slice_end = if idx == end_idx {
                (end - chunk_start) as usize
            } else {
                chunk_data.len()
            };
            let slice_end = slice_end.min(chunk_data.len());

            let slice = Bytes::copy_from_slice(&chunk_data[slice_start..slice_end]);
            if tx.send(Ok(slice)).await.is_err() {
                return;
            }
        }
    });

    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));

    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, &content_type)
        .header(header::ACCEPT_RANGES, "bytes");

    if is_range {
        let content_length = end - offset;
        response = response
            .header(header::CONTENT_LENGTH, content_length)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {}-{}/{}", offset, end - 1, total_size),
            );
    } else {
        response = response.header(header::CONTENT_LENGTH, total_size);
    }

    response.body(body).unwrap().into_response()
}

async fn head_media(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
) -> impl IntoResponse {
    let key = normalize_key(raw_key);
    match state.engine.get_manifest(&key) {
        Ok(Some(manifest)) => {
            let mut response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, &manifest.content_type)
                .header(header::CONTENT_LENGTH, manifest.total_size)
                .header(header::ACCEPT_RANGES, "bytes")
                .header("X-KaidaDB-Checksum", &manifest.checksum)
                .header("X-KaidaDB-Chunk-Count", manifest.chunk_count());

            for (k, v) in &manifest.metadata {
                response = response.header(format!("X-KaidaDB-Meta-{k}"), v);
            }

            response.body(Body::empty()).unwrap().into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn delete_media(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
) -> impl IntoResponse {
    let key = normalize_key(raw_key);
    // Invalidate cache
    if let Ok(Some(manifest)) = state.engine.get_manifest(&key) {
        for chunk_id in &manifest.chunks {
            state.cache.invalidate(chunk_id);
        }
    }

    match state.engine.delete(&key) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    prefix: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Serialize)]
struct ListResponse {
    items: Vec<MediaItem>,
    next_cursor: Option<String>,
}

#[derive(Serialize)]
struct MediaItem {
    key: String,
    total_size: u64,
    chunk_count: u32,
    content_type: String,
    checksum: String,
    created_at: i64,
}

async fn list_media(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> impl IntoResponse {
    let prefix = query.prefix.unwrap_or_default();
    let limit = query.limit.unwrap_or(100);
    let cursor = query.cursor.unwrap_or_default();

    match state.engine.list(&prefix, limit, &cursor) {
        Ok((manifests, next_cursor)) => {
            let items: Vec<MediaItem> = manifests
                .into_iter()
                .map(|m| {
                    let chunk_count = m.chunk_count();
                    MediaItem {
                        key: m.key,
                        total_size: m.total_size,
                        chunk_count,
                        content_type: m.content_type,
                        checksum: m.checksum,
                        created_at: m.created_at,
                    }
                })
                .collect();

            axum::Json(ListResponse {
                items,
                next_cursor,
            })
            .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_meta(
    State(state): State<AppState>,
    Path(raw_key): Path<String>,
) -> impl IntoResponse {
    let key = normalize_key(raw_key);
    match state.engine.get_manifest(&key) {
        Ok(Some(manifest)) => {
            let meta = serde_json::json!({
                "key": manifest.key,
                "total_size": manifest.total_size,
                "chunk_count": manifest.chunk_count(),
                "content_type": manifest.content_type,
                "checksum": manifest.checksum,
                "metadata": manifest.metadata,
                "created_at": manifest.created_at,
                "updated_at": manifest.updated_at,
            });
            axum::Json(meta).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

fn parse_range_header(headers: &HeaderMap, total_size: u64) -> (u64, u64, bool) {
    if total_size == 0 {
        return (0, 0, false);
    }
    if let Some(range_header) = headers.get(header::RANGE) {
        if let Ok(range_str) = range_header.to_str() {
            if let Some(bytes_range) = range_str.strip_prefix("bytes=") {
                // Handle suffix range: "bytes=-500" (last 500 bytes)
                if let Some(suffix) = bytes_range.strip_prefix('-') {
                    if let Ok(n) = suffix.parse::<u64>() {
                        if n == 0 {
                            return (0, 0, false);
                        }
                        let start = total_size.saturating_sub(n);
                        let length = total_size - start;
                        return (start, length, true);
                    }
                    return (0, 0, false);
                }
                let parts: Vec<&str> = bytes_range.splitn(2, '-').collect();
                if parts.len() == 2 {
                    let start: u64 = match parts[0].parse() {
                        Ok(s) => s,
                        Err(_) => return (0, 0, false),
                    };
                    if start >= total_size {
                        return (0, 0, false);
                    }
                    let end: u64 = if parts[1].is_empty() {
                        total_size - 1
                    } else {
                        match parts[1].parse::<u64>() {
                            Ok(e) => e.min(total_size - 1),
                            Err(_) => total_size - 1,
                        }
                    };
                    if end < start {
                        return (0, 0, false);
                    }
                    let length = end - start + 1;
                    return (start, length, true);
                }
            }
        }
    }
    (0, 0, false)
}

/// Fallback handler that routes `/v1/media/<key>` requests with slashes in the key
/// to the appropriate handler. Axum's `{key}` parameter only matches a single path
/// segment, but KaidaDB keys can contain slashes (e.g. `tv/show/s01/file.mp4`).
async fn media_fallback(
    State(state): State<AppState>,
    req: Request,
) -> impl IntoResponse {
    let path = req.uri().path();
    let headers = req.headers().clone();
    let method = req.method().clone();

    if let Some(raw_key) = path.strip_prefix("/v1/meta/") {
        let key = percent_decode_str(raw_key).decode_utf8_lossy().to_string();
        return get_meta(State(state), Path(key)).await.into_response();
    }

    let Some(raw_key) = path.strip_prefix("/v1/media/") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let key = percent_decode_str(raw_key).decode_utf8_lossy().to_string();

    match method {
        Method::GET => get_media(State(state), Path(key), headers).await.into_response(),
        Method::PUT => {
            let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
                Ok(b) => b,
                Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
            };
            put_media(State(state), Path(key), headers, body).await.into_response()
        }
        Method::HEAD => head_media(State(state), Path(key)).await.into_response(),
        Method::DELETE => delete_media(State(state), Path(key)).await.into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}
