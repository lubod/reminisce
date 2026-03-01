use thiserror::Error;

#[derive(Error, Debug)]
pub enum Np2pError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Crypto error: {0}")]
    Crypto(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Connect error: {0}")]
    Connect(#[from] quinn::ConnectError),

    #[error("Connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),

    #[error("Write error: {0}")]
    Write(#[from] quinn::WriteError),

    #[error("Read error: {0}")]
    Read(#[from] quinn::ReadExactError),

    #[error("Erasure coding error: {0}")]
    ErasureCoding(String),

    #[error("Identity error: {0}")]
    Identity(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Np2pError>;
