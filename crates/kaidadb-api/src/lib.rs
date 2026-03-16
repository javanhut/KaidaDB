pub mod grpc;
pub mod rest;

pub mod proto {
    tonic::include_proto!("kaidadb");
}
