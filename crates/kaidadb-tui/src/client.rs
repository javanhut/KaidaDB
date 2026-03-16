use tonic::transport::Channel;

pub mod proto {
    tonic::include_proto!("kaidadb");
}

pub use proto::kaida_db_client::KaidaDbClient;
pub use proto::*;

pub async fn connect(addr: &str) -> Result<KaidaDbClient<Channel>, tonic::transport::Error> {
    KaidaDbClient::connect(addr.to_string()).await
}

pub fn guess_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("mp4") => "video/mp4",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}
