use std::sync::Arc;

use kaidadb_common::config::StreamingConfig;
use kaidadb_storage::StorageEngine;

/// Information about a single variant (quality level) of a stream.
#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub variant_id: String,
    pub init_key: String,
    pub codec: String,
    pub bandwidth: u64,
    pub width: u32,
    pub height: u32,
    pub media_type: String, // "video", "audio", "subtitle"
    pub language: String,
    pub frame_rate: String,
    pub sample_rate: String,
    pub channels: String,
}

/// Information about a single segment.
#[derive(Debug, Clone)]
pub struct SegmentInfo {
    pub key: String,
    pub index: u64,
    pub duration: f64,
}

/// Discover all variants for a given stream by listing init segments.
pub fn discover_variants(
    engine: &Arc<StorageEngine>,
    config: &StreamingConfig,
    stream_id: &str,
) -> Result<Vec<VariantInfo>, String> {
    let prefix = format!("{}{}variants/", config.stream_prefix, stream_id);
    let (manifests, _) = engine
        .list(&prefix, 10000, "")
        .map_err(|e| e.to_string())?;

    let mut variants: Vec<VariantInfo> = Vec::new();
    let mut seen_variants = std::collections::HashSet::new();

    for manifest in &manifests {
        // Only look at init.mp4 files to identify variants
        if !manifest.key.ends_with("/init.mp4") {
            continue;
        }

        // Extract variant_id from key: streams/{id}/variants/{variant_id}/init.mp4
        let after_prefix = match manifest.key.strip_prefix(&prefix) {
            Some(s) => s,
            None => continue,
        };
        let variant_id = match after_prefix.strip_suffix("/init.mp4") {
            Some(s) => s.to_string(),
            None => continue,
        };

        if !seen_variants.insert(variant_id.clone()) {
            continue;
        }

        let meta = &manifest.metadata;
        variants.push(VariantInfo {
            variant_id: variant_id.clone(),
            init_key: manifest.key.clone(),
            codec: meta.get("codec").cloned().unwrap_or_default(),
            bandwidth: meta
                .get("bandwidth")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            width: meta
                .get("width")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            height: meta
                .get("height")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            media_type: meta
                .get("media-type")
                .cloned()
                .unwrap_or_else(|| "video".to_string()),
            language: meta.get("language").cloned().unwrap_or_default(),
            frame_rate: meta.get("frame-rate").cloned().unwrap_or_default(),
            sample_rate: meta.get("sample-rate").cloned().unwrap_or_default(),
            channels: meta.get("channels").cloned().unwrap_or_default(),
        });
    }

    variants.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));
    Ok(variants)
}

/// Discover all segments for a specific variant.
pub fn discover_segments(
    engine: &Arc<StorageEngine>,
    config: &StreamingConfig,
    stream_id: &str,
    variant_id: &str,
) -> Result<Vec<SegmentInfo>, String> {
    let prefix = format!(
        "{}{}variants/{}/seg-",
        config.stream_prefix, stream_id, variant_id
    );
    let (manifests, _) = engine
        .list(&prefix, 100000, "")
        .map_err(|e| e.to_string())?;

    let mut segments: Vec<SegmentInfo> = Vec::new();

    for manifest in &manifests {
        let meta = &manifest.metadata;
        let index = meta
            .get("segment-index")
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                // Fallback: extract from key name (seg-000042.m4s -> 42)
                extract_segment_index(&manifest.key).unwrap_or(segments.len() as u64)
            });
        let duration = meta
            .get("segment-duration")
            .and_then(|s| s.parse().ok())
            .unwrap_or(4.0);

        segments.push(SegmentInfo {
            key: manifest.key.clone(),
            index,
            duration,
        });
    }

    segments.sort_by_key(|s| s.index);
    Ok(segments)
}

/// Extract segment index from key like "streams/id/variants/720p/seg-000042.m4s"
fn extract_segment_index(key: &str) -> Option<u64> {
    let filename = key.rsplit('/').next()?;
    let num_part = filename.strip_prefix("seg-")?;
    let num_str = num_part.split('.').next()?;
    num_str.parse().ok()
}

/// Compute the maximum segment duration across all segments.
fn max_target_duration(segments: &[SegmentInfo], default: f64) -> u64 {
    segments
        .iter()
        .map(|s| s.duration.ceil() as u64)
        .max()
        .unwrap_or(default.ceil() as u64)
}

/// Build the media URL for a given key.
fn media_url(base_url: &str, key: &str) -> String {
    if base_url.is_empty() {
        format!("/v1/media/{}", key)
    } else {
        let base = base_url.trim_end_matches('/');
        format!("{}/v1/media/{}", base, key)
    }
}

// ─── HLS ────────────────────────────────────────────────────────────────────

/// Generate an HLS master playlist (.m3u8) listing all variants.
pub fn generate_hls_master(
    variants: &[VariantInfo],
    stream_id: &str,
    config: &StreamingConfig,
) -> String {
    let mut out = String::new();
    out.push_str("#EXTM3U\n");
    out.push_str("#EXT-X-VERSION:7\n\n");

    // Separate audio-only and video variants
    let audio_variants: Vec<_> = variants.iter().filter(|v| v.media_type == "audio").collect();
    let video_variants: Vec<_> = variants.iter().filter(|v| v.media_type == "video").collect();
    let subtitle_variants: Vec<_> = variants
        .iter()
        .filter(|v| v.media_type == "subtitle")
        .collect();

    // Emit EXT-X-MEDIA for audio tracks (if there are separate audio variants)
    for av in &audio_variants {
        let lang = if av.language.is_empty() {
            "und"
        } else {
            &av.language
        };
        let name = if av.language.is_empty() {
            &av.variant_id
        } else {
            &av.language
        };
        out.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"{}\",LANGUAGE=\"{}\",DEFAULT=YES,URI=\"{}\"\n",
            name,
            lang,
            variant_playlist_url(&config.base_url, stream_id, &av.variant_id),
        ));
    }

    // Emit EXT-X-MEDIA for subtitle tracks
    for sv in &subtitle_variants {
        let lang = if sv.language.is_empty() {
            "und"
        } else {
            &sv.language
        };
        out.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"{}\",LANGUAGE=\"{}\",DEFAULT=NO,URI=\"{}\"\n",
            lang,
            lang,
            variant_playlist_url(&config.base_url, stream_id, &sv.variant_id),
        ));
    }

    if !audio_variants.is_empty() || !subtitle_variants.is_empty() {
        out.push('\n');
    }

    // Emit EXT-X-STREAM-INF for each video variant
    for vv in &video_variants {
        let mut attrs = format!("BANDWIDTH={}", vv.bandwidth);
        if vv.width > 0 && vv.height > 0 {
            attrs.push_str(&format!(",RESOLUTION={}x{}", vv.width, vv.height));
        }
        if !vv.codec.is_empty() {
            attrs.push_str(&format!(",CODECS=\"{}\"", vv.codec));
        }
        if !vv.frame_rate.is_empty() {
            attrs.push_str(&format!(",FRAME-RATE={}", vv.frame_rate));
        }
        if !audio_variants.is_empty() {
            attrs.push_str(",AUDIO=\"audio\"");
        }
        if !subtitle_variants.is_empty() {
            attrs.push_str(",SUBTITLES=\"subs\"");
        }
        out.push_str(&format!("#EXT-X-STREAM-INF:{}\n", attrs));
        out.push_str(&variant_playlist_url(
            &config.base_url,
            stream_id,
            &vv.variant_id,
        ));
        out.push('\n');
    }

    // If there are ONLY audio variants (music/podcast), list them as stream-inf
    if video_variants.is_empty() && !audio_variants.is_empty() {
        for av in &audio_variants {
            let mut attrs = format!("BANDWIDTH={}", av.bandwidth);
            if !av.codec.is_empty() {
                attrs.push_str(&format!(",CODECS=\"{}\"", av.codec));
            }
            out.push_str(&format!("#EXT-X-STREAM-INF:{}\n", attrs));
            out.push_str(&variant_playlist_url(
                &config.base_url,
                stream_id,
                &av.variant_id,
            ));
            out.push('\n');
        }
    }

    out
}

/// Generate an HLS media playlist (.m3u8) for a single variant.
pub fn generate_hls_media(
    segments: &[SegmentInfo],
    init_key: &str,
    config: &StreamingConfig,
) -> String {
    let mut out = String::new();
    out.push_str("#EXTM3U\n");
    out.push_str("#EXT-X-VERSION:7\n");

    let target_dur = max_target_duration(segments, config.target_duration);
    out.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", target_dur));
    out.push_str("#EXT-X-MEDIA-SEQUENCE:0\n");

    // fMP4 initialization segment
    out.push_str(&format!(
        "#EXT-X-MAP:URI=\"{}\"\n",
        media_url(&config.base_url, init_key)
    ));

    for seg in segments {
        out.push_str(&format!("#EXTINF:{:.3},\n", seg.duration));
        out.push_str(&media_url(&config.base_url, &seg.key));
        out.push('\n');
    }

    if config.vod_mode {
        out.push_str("#EXT-X-ENDLIST\n");
    }

    out
}

fn variant_playlist_url(base_url: &str, stream_id: &str, variant_id: &str) -> String {
    if base_url.is_empty() {
        format!(
            "/v1/streams/{}/variant/{}/playlist.m3u8",
            stream_id, variant_id
        )
    } else {
        let base = base_url.trim_end_matches('/');
        format!(
            "{}/v1/streams/{}/variant/{}/playlist.m3u8",
            base, stream_id, variant_id
        )
    }
}

// ─── DASH ───────────────────────────────────────────────────────────────────

/// Generate a DASH MPD manifest for a stream.
pub fn generate_dash_mpd(
    variants: &[VariantInfo],
    segments_by_variant: &[(String, Vec<SegmentInfo>)],
    config: &StreamingConfig,
) -> String {
    // Compute total duration from the first variant's segments
    let total_duration_secs: f64 = segments_by_variant
        .first()
        .map(|(_, segs)| segs.iter().map(|s| s.duration).sum())
        .unwrap_or(0.0);
    let duration_iso = format_iso_duration(total_duration_secs);

    let target_dur = config.target_duration;

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"{}\" minBufferTime=\"PT2S\" profiles=\"urn:mpeg:dash:profile:isoff-on-demand:2011\">\n",
        duration_iso
    ));
    out.push_str("  <Period>\n");

    // Group variants by media type for adaptation sets
    let video_variants: Vec<_> = variants.iter().filter(|v| v.media_type == "video").collect();
    let audio_variants: Vec<_> = variants.iter().filter(|v| v.media_type == "audio").collect();

    // Video AdaptationSet
    if !video_variants.is_empty() {
        out.push_str(
            "    <AdaptationSet mimeType=\"video/mp4\" segmentAlignment=\"true\" startWithSAP=\"1\">\n",
        );
        for vv in &video_variants {
            let segments = segments_by_variant
                .iter()
                .find(|(id, _)| id == &vv.variant_id)
                .map(|(_, s)| s.as_slice())
                .unwrap_or(&[]);

            out.push_str(&format!(
                "      <Representation id=\"{}\" bandwidth=\"{}\"",
                vv.variant_id, vv.bandwidth
            ));
            if vv.width > 0 && vv.height > 0 {
                out.push_str(&format!(" width=\"{}\" height=\"{}\"", vv.width, vv.height));
            }
            if !vv.codec.is_empty() {
                out.push_str(&format!(" codecs=\"{}\"", vv.codec));
            }
            if !vv.frame_rate.is_empty() {
                out.push_str(&format!(" frameRate=\"{}\"", vv.frame_rate));
            }
            out.push_str(">\n");
            out.push_str(&dash_segment_template(
                &vv.init_key,
                &vv.variant_id,
                config,
                segments,
                target_dur,
            ));
            out.push_str("      </Representation>\n");
        }
        out.push_str("    </AdaptationSet>\n");
    }

    // Audio AdaptationSet
    if !audio_variants.is_empty() {
        out.push_str(
            "    <AdaptationSet mimeType=\"audio/mp4\" segmentAlignment=\"true\" startWithSAP=\"1\">\n",
        );
        for av in &audio_variants {
            let segments = segments_by_variant
                .iter()
                .find(|(id, _)| id == &av.variant_id)
                .map(|(_, s)| s.as_slice())
                .unwrap_or(&[]);

            out.push_str(&format!(
                "      <Representation id=\"{}\" bandwidth=\"{}\"",
                av.variant_id, av.bandwidth
            ));
            if !av.codec.is_empty() {
                out.push_str(&format!(" codecs=\"{}\"", av.codec));
            }
            if !av.sample_rate.is_empty() {
                out.push_str(&format!(" audioSamplingRate=\"{}\"", av.sample_rate));
            }
            out.push_str(">\n");
            if !av.channels.is_empty() {
                out.push_str(&format!(
                    "        <AudioChannelConfiguration schemeIdUri=\"urn:mpeg:dash:23003:3:audio_channel_configuration:2011\" value=\"{}\"/>\n",
                    av.channels
                ));
            }
            out.push_str(&dash_segment_template(
                &av.init_key,
                &av.variant_id,
                config,
                segments,
                target_dur,
            ));
            out.push_str("      </Representation>\n");
        }
        out.push_str("    </AdaptationSet>\n");
    }

    out.push_str("  </Period>\n");
    out.push_str("</MPD>\n");
    out
}

fn dash_segment_template(
    init_key: &str,
    _variant_id: &str,
    config: &StreamingConfig,
    segments: &[SegmentInfo],
    default_duration: f64,
) -> String {
    let init_url = media_url(&config.base_url, init_key);

    // Check if all segments have uniform duration
    let all_uniform = segments.len() <= 1
        || segments
            .windows(2)
            .all(|w| (w[0].duration - w[1].duration).abs() < 0.01);

    if all_uniform && !segments.is_empty() {
        let duration_ms = (segments[0].duration * 1000.0) as u64;
        format!(
            "        <SegmentTemplate timescale=\"1000\" duration=\"{}\" startNumber=\"0\" initialization=\"{}\" media=\"{}\"/>\n",
            duration_ms,
            init_url,
            segment_media_template(segments, config),
        )
    } else {
        // Use SegmentTimeline for variable-duration segments
        let mut s = format!(
            "        <SegmentTemplate timescale=\"1000\" startNumber=\"0\" initialization=\"{}\">\n",
            init_url
        );
        s.push_str("          <SegmentTimeline>\n");
        for seg in segments {
            let dur_ms = (seg.duration * 1000.0) as u64;
            s.push_str(&format!("            <S d=\"{}\"/>\n", dur_ms));
        }
        s.push_str("          </SegmentTimeline>\n");
        s.push_str("        </SegmentTemplate>\n");

        // Also provide SegmentList with explicit URLs since template pattern
        // may not match our key naming
        let _ = default_duration; // used above via config
        s
    }
}

fn segment_media_template(segments: &[SegmentInfo], config: &StreamingConfig) -> String {
    // If segments follow a predictable pattern, use a template.
    // Otherwise, fall back to individual URLs (handled by SegmentList).
    if let Some(first) = segments.first() {
        // Try to build a template from the first segment's key
        // e.g., "streams/movie/variants/720p/seg-000000.m4s"
        // We want to replace the number part with $Number%06d$
        if let Some(seg_pos) = first.key.rfind("/seg-") {
            let base_path = &first.key[..seg_pos + 5]; // includes "/seg-"
            let ext = first
                .key
                .rsplit('.')
                .next()
                .unwrap_or("m4s");
            let url = media_url(&config.base_url, &format!("{}$Number%06d$.{}", base_path, ext));
            return url;
        }
    }
    String::new()
}

fn format_iso_duration(seconds: f64) -> String {
    let hours = (seconds / 3600.0) as u64;
    let mins = ((seconds % 3600.0) / 60.0) as u64;
    let secs = seconds % 60.0;

    if hours > 0 {
        format!("PT{}H{}M{:.1}S", hours, mins, secs)
    } else if mins > 0 {
        format!("PT{}M{:.1}S", mins, secs)
    } else {
        format!("PT{:.1}S", secs)
    }
}

// ─── Stream listing ─────────────────────────────────────────────────────────

/// Information about a stream for listing purposes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StreamListItem {
    pub stream_id: String,
    pub variant_count: usize,
}

/// List unique stream IDs by scanning stored keys.
pub fn list_streams(
    engine: &Arc<StorageEngine>,
    config: &StreamingConfig,
    prefix: &str,
    limit: usize,
    cursor: &str,
) -> Result<(Vec<StreamListItem>, Option<String>), String> {
    let scan_prefix = if prefix.is_empty() {
        config.stream_prefix.clone()
    } else {
        format!("{}{}", config.stream_prefix, prefix)
    };

    let (manifests, _) = engine
        .list(&scan_prefix, 100000, cursor)
        .map_err(|e| e.to_string())?;

    let mut stream_ids: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for manifest in &manifests {
        // Extract stream_id from key: streams/{stream_id}/variants/...
        if let Some(after) = manifest.key.strip_prefix(&config.stream_prefix) {
            if let Some(slash_pos) = after.find('/') {
                let stream_id = &after[..slash_pos];
                *stream_ids.entry(stream_id.to_string()).or_insert(0) += 1;
            }
        }
    }

    let items: Vec<StreamListItem> = stream_ids
        .into_iter()
        .take(limit)
        .map(|(stream_id, variant_count)| StreamListItem {
            stream_id,
            variant_count,
        })
        .collect();

    let next_cursor = if items.len() == limit {
        items.last().map(|i| {
            format!(
                "{}{}{}",
                config.stream_prefix,
                i.stream_id,
                // Append a char beyond '/' to skip past this stream's entries
                "0"
            )
        })
    } else {
        None
    };

    Ok((items, next_cursor))
}

/// Delete all segments and variants for a stream.
pub fn delete_stream(
    engine: &Arc<StorageEngine>,
    config: &StreamingConfig,
    stream_id: &str,
) -> Result<(u32, u32), String> {
    let prefix = format!("{}{}/", config.stream_prefix, stream_id);
    let mut variants_deleted = 0u32;
    let mut segments_deleted = 0u32;

    // Paginate through all keys under this stream
    let mut cursor = String::new();
    loop {
        let (manifests, next_cursor) = engine
            .list(&prefix, 1000, &cursor)
            .map_err(|e| e.to_string())?;

        if manifests.is_empty() {
            break;
        }

        for manifest in &manifests {
            engine.delete(&manifest.key).map_err(|e| e.to_string())?;
            if manifest.key.ends_with("/init.mp4") {
                variants_deleted += 1;
            } else {
                segments_deleted += 1;
            }
        }

        match next_cursor {
            Some(c) => cursor = c,
            None => break,
        }
    }

    Ok((variants_deleted, segments_deleted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaidadb_common::config::StreamingConfig;

    fn test_config() -> StreamingConfig {
        StreamingConfig {
            target_duration: 4.0,
            base_url: String::new(),
            stream_prefix: "streams/".to_string(),
            vod_mode: true,
        }
    }

    #[test]
    fn test_extract_segment_index() {
        assert_eq!(extract_segment_index("streams/id/variants/720p/seg-000042.m4s"), Some(42));
        assert_eq!(extract_segment_index("seg-000000.ts"), Some(0));
        assert_eq!(extract_segment_index("streams/id/init.mp4"), None);
    }

    #[test]
    fn test_format_iso_duration() {
        assert_eq!(format_iso_duration(30.0), "PT30.0S");
        assert_eq!(format_iso_duration(90.0), "PT1M30.0S");
        assert_eq!(format_iso_duration(3661.5), "PT1H1M1.5S");
    }

    #[test]
    fn test_media_url() {
        assert_eq!(media_url("", "streams/id/seg-000.m4s"), "/v1/media/streams/id/seg-000.m4s");
        assert_eq!(
            media_url("https://cdn.example.com", "streams/id/seg-000.m4s"),
            "https://cdn.example.com/v1/media/streams/id/seg-000.m4s"
        );
        assert_eq!(
            media_url("https://cdn.example.com/", "streams/id/seg-000.m4s"),
            "https://cdn.example.com/v1/media/streams/id/seg-000.m4s"
        );
    }

    #[test]
    fn test_generate_hls_master_video() {
        let config = test_config();
        let variants = vec![
            VariantInfo {
                variant_id: "1080p".into(),
                init_key: "streams/movie/variants/1080p/init.mp4".into(),
                codec: "avc1.640028".into(),
                bandwidth: 5000000,
                width: 1920,
                height: 1080,
                media_type: "video".into(),
                language: String::new(),
                frame_rate: "30".into(),
                sample_rate: String::new(),
                channels: String::new(),
            },
            VariantInfo {
                variant_id: "720p".into(),
                init_key: "streams/movie/variants/720p/init.mp4".into(),
                codec: "avc1.64001f".into(),
                bandwidth: 2500000,
                width: 1280,
                height: 720,
                media_type: "video".into(),
                language: String::new(),
                frame_rate: "30".into(),
                sample_rate: String::new(),
                channels: String::new(),
            },
        ];

        let playlist = generate_hls_master(&variants, "movie", &config);
        assert!(playlist.contains("#EXTM3U"));
        assert!(playlist.contains("#EXT-X-VERSION:7"));
        assert!(playlist.contains("BANDWIDTH=5000000"));
        assert!(playlist.contains("RESOLUTION=1920x1080"));
        assert!(playlist.contains("CODECS=\"avc1.640028\""));
        assert!(playlist.contains("/v1/streams/movie/variant/1080p/playlist.m3u8"));
        assert!(playlist.contains("/v1/streams/movie/variant/720p/playlist.m3u8"));
    }

    #[test]
    fn test_generate_hls_master_audio_only() {
        let config = test_config();
        let variants = vec![VariantInfo {
            variant_id: "aac-128k".into(),
            init_key: "streams/song/variants/aac-128k/init.mp4".into(),
            codec: "mp4a.40.2".into(),
            bandwidth: 128000,
            width: 0,
            height: 0,
            media_type: "audio".into(),
            language: "en".into(),
            frame_rate: String::new(),
            sample_rate: "44100".into(),
            channels: "2".into(),
        }];

        let playlist = generate_hls_master(&variants, "song", &config);
        assert!(playlist.contains("BANDWIDTH=128000"));
        assert!(playlist.contains("CODECS=\"mp4a.40.2\""));
    }

    #[test]
    fn test_generate_hls_media() {
        let config = test_config();
        let segments = vec![
            SegmentInfo {
                key: "streams/movie/variants/720p/seg-000000.m4s".into(),
                index: 0,
                duration: 4.0,
            },
            SegmentInfo {
                key: "streams/movie/variants/720p/seg-000001.m4s".into(),
                index: 1,
                duration: 4.0,
            },
            SegmentInfo {
                key: "streams/movie/variants/720p/seg-000002.m4s".into(),
                index: 2,
                duration: 3.5,
            },
        ];

        let playlist = generate_hls_media(
            &segments,
            "streams/movie/variants/720p/init.mp4",
            &config,
        );
        assert!(playlist.contains("#EXTM3U"));
        assert!(playlist.contains("#EXT-X-TARGETDURATION:4"));
        assert!(playlist.contains("#EXT-X-MAP:URI=\"/v1/media/streams/movie/variants/720p/init.mp4\""));
        assert!(playlist.contains("#EXTINF:4.000,"));
        assert!(playlist.contains("#EXTINF:3.500,"));
        assert!(playlist.contains("/v1/media/streams/movie/variants/720p/seg-000000.m4s"));
        assert!(playlist.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn test_generate_hls_media_live_mode() {
        let mut config = test_config();
        config.vod_mode = false;

        let segments = vec![SegmentInfo {
            key: "streams/live/variants/720p/seg-000000.m4s".into(),
            index: 0,
            duration: 4.0,
        }];

        let playlist = generate_hls_media(
            &segments,
            "streams/live/variants/720p/init.mp4",
            &config,
        );
        assert!(!playlist.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn test_max_target_duration() {
        let segments = vec![
            SegmentInfo { key: String::new(), index: 0, duration: 4.0 },
            SegmentInfo { key: String::new(), index: 1, duration: 4.5 },
            SegmentInfo { key: String::new(), index: 2, duration: 3.2 },
        ];
        assert_eq!(max_target_duration(&segments, 4.0), 5); // ceil(4.5) = 5
        assert_eq!(max_target_duration(&[], 4.0), 4);
    }
}
