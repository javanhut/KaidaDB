use std::future::IntoFuture;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::signal;
use tonic::transport::Server as TonicServer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

use kaidadb_api::grpc::KaidaDbGrpc;
use kaidadb_api::proto::kaida_db_server::KaidaDbServer;
use kaidadb_api::rest;
use kaidadb_cache::ChunkCache;
use kaidadb_common::{server_key, KaidaDbConfig};
use kaidadb_storage::StorageEngine;

/// Mirror `kaidadb-ctl`'s log location so users get one canonical log path.
fn log_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("kaidadb");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".local/state/kaidadb");
        }
    }
    PathBuf::from("/tmp/kaidadb")
}

#[derive(Parser)]
#[command(name = "kaidadb-server", version, about = "KaidaDB media database server")]
struct Args {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,

    /// Regenerate the server access key and exit
    #[arg(long)]
    regenerate_key: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdout → systemd journal; file → persisted post-mortem log at
    // $XDG_STATE_HOME/kaidadb/kaidadb.log, matching the path kaidadb-ctl uses.
    let log_dir = log_dir();
    let file_guard = match std::fs::create_dir_all(&log_dir) {
        Ok(()) => {
            let appender =
                tracing_appender::rolling::daily(&log_dir, "kaidadb.log");
            let (nb, guard) = tracing_appender::non_blocking(appender);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(nb)
                .with_ansi(false);
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "kaidadb=info,tower_http=info".into()),
                )
                .with(tracing_subscriber::fmt::layer().boxed())
                .with(file_layer.boxed())
                .init();
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "kaidadb=info,tower_http=info".into()),
                )
                .with(tracing_subscriber::fmt::layer())
                .init();
            tracing::warn!(%e, path=%log_dir.display(), "could not create log dir; stdout only");
            None
        }
    };

    let args = Args::parse();
    let config = KaidaDbConfig::load(args.config.as_deref())?;
    config.validate()?;

    // Handle --regenerate-key
    if args.regenerate_key {
        let plaintext = server_key::regenerate_key(&config.data_dir)?;
        println!("New server key: {plaintext}");
        println!("Save this key — you'll need it for remote CLI/TUI access.");
        return Ok(());
    }

    tracing::info!(?config, "starting KaidaDB server");

    // Load or generate server key
    let (key_hash, plaintext) = server_key::load_or_create_key(&config.data_dir)?;
    if let Some(pt) = plaintext {
        tracing::info!("========================================");
        tracing::info!("  Generated new server key: {}", pt);
        tracing::info!("  Save this key for remote CLI/TUI access.");
        tracing::info!("  To regenerate: kaidadb-server --regenerate-key");
        tracing::info!("========================================");
    }
    let key_hash = Arc::new(key_hash);

    // Initialize storage engine
    let engine = Arc::new(StorageEngine::open(&config.data_dir, config.storage.chunk_size)?);

    // Initialize cache
    let cache = Arc::new(ChunkCache::new(config.cache.max_size));

    // gRPC server
    let grpc_addr = config.grpc_addr.parse()?;
    let grpc_service = KaidaDbGrpc::new(
        engine.clone(),
        cache.clone(),
        config.streaming.clone(),
        key_hash.clone(),
    );

    let grpc_server = TonicServer::builder()
        .add_service(KaidaDbServer::new(grpc_service))
        .serve(grpc_addr);

    // REST server
    let rest_state = rest::AppState {
        engine: engine.clone(),
        cache: cache.clone(),
        streaming: config.streaming.clone(),
        server_key_hash: key_hash,
    };
    let rest_app = rest::router(rest_state);
    let rest_addr: std::net::SocketAddr = config.rest_addr.parse()?;
    let rest_listener = tokio::net::TcpListener::bind(rest_addr).await?;

    tracing::info!(%grpc_addr, %rest_addr, "server listening");

    // Run both servers concurrently, shut down on ctrl-c
    tokio::select! {
        result = grpc_server => {
            if let Err(e) = result {
                tracing::error!(%e, "gRPC server error");
            }
        }
        result = axum::serve(
            rest_listener,
            rest_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        ).into_future() => {
            if let Err(e) = result {
                tracing::error!(%e, "REST server error");
            }
        }
        _ = signal::ctrl_c() => {
            tracing::info!("shutting down");
        }
    }

    // Keep the non-blocking file appender guard alive for the whole run.
    drop(file_guard);
    Ok(())
}
