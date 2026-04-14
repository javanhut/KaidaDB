pub mod grpc;
pub mod rest;
pub mod streaming;

pub mod proto {
    tonic::include_proto!("kaidadb");
}
