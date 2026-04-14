use std::future::IntoFuture;
use std::sync::Arc;

use clap::Parser;
use tokio::signal;
use tonic::transport::Server as TonicServer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use kaidadb_api::grpc::KaidaDbGrpc;
use kaidadb_api::proto::kaida_db_server::KaidaDbServer;
use kaidadb_api::rest;
use kaidadb_cache::ChunkCache;
use kaidadb_common::KaidaDbConfig;
use kaidadb_storage::StorageEngine;

#[derive(Parser)]
#[command(name = "kaidadb-server", version, about = "KaidaDB media database server")]
struct Args {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "kaidadb=info,tower_http=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();
    let config = KaidaDbConfig::load(args.config.as_deref())?;
    config.validate()?;

    tracing::info!(?config, "starting KaidaDB server");

    // Initialize storage engine
    let engine = Arc::new(StorageEngine::open(&config.data_dir, config.storage.chunk_size)?);

    // Initialize cache
    let cache = Arc::new(ChunkCache::new(config.cache.max_size));

    // gRPC server
    let grpc_addr = config.grpc_addr.parse()?;
    let grpc_service = KaidaDbGrpc::new(engine.clone(), cache.clone(), config.streaming.clone());

    let grpc_server = TonicServer::builder()
        .add_service(KaidaDbServer::new(grpc_service))
        .serve(grpc_addr);

    // REST server
    let rest_state = rest::AppState {
        engine: engine.clone(),
        cache: cache.clone(),
        streaming: config.streaming.clone(),
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
        result = axum::serve(rest_listener, rest_app).into_future() => {
            if let Err(e) = result {
                tracing::error!(%e, "REST server error");
            }
        }
        _ = signal::ctrl_c() => {
            tracing::info!("shutting down");
        }
    }

    Ok(())
}
