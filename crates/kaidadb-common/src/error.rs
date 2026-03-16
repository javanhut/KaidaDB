use thiserror::Error;

pub type Result<T> = std::result::Result<T, KaidaDbError>;

#[derive(Error, Debug)]
pub enum KaidaDbError {
    #[error("key not found: {0}")]
    NotFound(String),

    #[error("key already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid key: {0}")]
    InvalidKey(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("chunk integrity error: chunk {chunk_id} — {detail}")]
    ChunkIntegrity { chunk_id: String, detail: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("invalid chunk format: {0}")]
    InvalidChunkFormat(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl From<bincode::Error> for KaidaDbError {
    fn from(e: bincode::Error) -> Self {
        KaidaDbError::Serialization(e.to_string())
    }
}
