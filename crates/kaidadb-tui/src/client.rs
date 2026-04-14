use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

pub mod proto {
    tonic::include_proto!("kaidadb");
}

pub use proto::kaida_db_client::KaidaDbClient;
pub use proto::*;

pub type AuthInterceptor = Box<dyn FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> + Send>;
pub type AuthClient = KaidaDbClient<InterceptedService<Channel, AuthInterceptor>>;

pub async fn connect(
    addr: &str,
    server_pass: Option<String>,
) -> Result<AuthClient, tonic::transport::Error> {
    let channel = Channel::from_shared(addr.to_string())
        .unwrap()
        .connect()
        .await?;

    let interceptor: AuthInterceptor = if let Some(pass) = server_pass {
        Box::new(move |mut req: tonic::Request<()>| {
            req.metadata_mut().insert(
                "x-server-pass",
                pass.parse().map_err(|_| {
                    tonic::Status::invalid_argument("invalid server password characters")
                })?,
            );
            Ok(req)
        })
    } else {
        Box::new(|req: tonic::Request<()>| Ok(req))
    };

    Ok(KaidaDbClient::with_interceptor(channel, interceptor))
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
