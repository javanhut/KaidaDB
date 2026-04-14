use anyhow::Result;
use clap::{Parser, Subcommand};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

pub mod proto {
    tonic::include_proto!("kaidadb");
}

use proto::kaida_db_client::KaidaDbClient;
use proto::*;

type AuthInterceptor = Box<dyn FnMut(tonic::Request<()>) -> std::result::Result<tonic::Request<()>, tonic::Status> + Send>;
type AuthClient = KaidaDbClient<InterceptedService<Channel, AuthInterceptor>>;

#[derive(Parser)]
#[command(name = "kaidadb-cli", version, about = "KaidaDB CLI client")]
struct Cli {
    /// Server gRPC address
    #[arg(short, long, default_value = "http://localhost:50051")]
    addr: String,

    /// Server password for remote access
    #[arg(long)]
    server_pass: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Store a file as media
    Store {
        /// Media key
        key: String,
        /// Path to file
        file: String,
        /// Content type (auto-detected if not specified)
        #[arg(short, long)]
        content_type: Option<String>,
    },
    /// Get media and write to file or stdout
    Get {
        /// Media key
        key: String,
        /// Output file path (stdout if not specified)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Get media metadata
    Meta {
        /// Media key
        key: String,
    },
    /// Delete media
    Delete {
        /// Media key
        key: String,
    },
    /// List media keys
    List {
        /// Key prefix filter
        #[arg(short, long, default_value = "")]
        prefix: String,
        /// Maximum number of results
        #[arg(short, long, default_value = "100")]
        limit: u32,
    },
    /// Health check
    Health,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let channel = Channel::from_shared(cli.addr)?.connect().await?;
    let interceptor: AuthInterceptor = if let Some(ref pass) = cli.server_pass {
        let pass = pass.clone();
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
    let mut client: AuthClient = KaidaDbClient::with_interceptor(channel, interceptor);

    match cli.command {
        Commands::Store {
            key,
            file,
            content_type,
        } => {
            let data = tokio::fs::read(&file).await?;
            let ct = content_type.unwrap_or_else(|| guess_content_type(&file));

            let header = StoreMediaRequest {
                request: Some(store_media_request::Request::Header(StoreMediaHeader {
                    key: key.clone(),
                    content_type: ct,
                    metadata: Default::default(),
                })),
            };

            // Build chunk stream: header first, then data chunks
            let chunk_size = 2 * 1024 * 1024; // 2 MiB
            let mut messages = vec![header];
            for chunk in data.chunks(chunk_size) {
                messages.push(StoreMediaRequest {
                    request: Some(store_media_request::Request::ChunkData(chunk.to_vec())),
                });
            }

            let response = client
                .store_media(tokio_stream::iter(messages))
                .await?
                .into_inner();

            println!(
                "Stored '{}': {} bytes, {} chunks, checksum: {}",
                response.key, response.total_size, response.chunk_count, response.checksum
            );
        }

        Commands::Get { key, output } => {
            let request = StreamMediaRequest {
                key: key.clone(),
                offset: 0,
                length: 0,
            };

            let mut stream = client.stream_media(request).await?.into_inner();
            let mut data = Vec::new();

            while let Some(chunk) = stream.message().await? {
                data.extend_from_slice(&chunk.data);
            }

            if let Some(path) = output {
                tokio::fs::write(&path, &data).await?;
                println!("Written {} bytes to {}", data.len(), path);
            } else {
                use std::io::Write;
                std::io::stdout().write_all(&data)?;
            }
        }

        Commands::Meta { key } => {
            let response = client
                .get_media_meta(GetMediaMetaRequest { key })
                .await?
                .into_inner();

            println!("Key:          {}", response.key);
            println!("Size:         {} bytes", response.total_size);
            println!("Chunks:       {}", response.chunk_count);
            println!("Content-Type: {}", response.content_type);
            println!("Checksum:     {}", response.checksum);
            if !response.metadata.is_empty() {
                println!("Metadata:");
                for (k, v) in &response.metadata {
                    println!("  {}: {}", k, v);
                }
            }
        }

        Commands::Delete { key } => {
            let response = client
                .delete_media(DeleteMediaRequest { key: key.clone() })
                .await?
                .into_inner();

            if response.deleted {
                println!("Deleted '{}'", key);
            } else {
                println!("Key '{}' not found", key);
            }
        }

        Commands::List { prefix, limit } => {
            let response = client
                .list_media(ListMediaRequest {
                    prefix,
                    limit,
                    cursor: String::new(),
                })
                .await?
                .into_inner();

            if response.items.is_empty() {
                println!("No media found");
            } else {
                for item in &response.items {
                    println!(
                        "{:40} {:>12} bytes  {}",
                        item.key, item.total_size, item.content_type
                    );
                }
                if !response.next_cursor.is_empty() {
                    println!("(more results available)");
                }
            }
        }

        Commands::Health => {
            let response = client
                .health_check(HealthCheckRequest {})
                .await?
                .into_inner();

            println!("Status:  {}", response.status);
            println!("Version: {}", response.version);
        }
    }

    Ok(())
}

fn guess_content_type(path: &str) -> String {
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
    .to_string()
}
