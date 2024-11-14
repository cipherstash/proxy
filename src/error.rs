use cipherstash_client::zerokms;
use std::io;
use thiserror::Error;
use tokio::time::error::Elapsed;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Client(#[from] cipherstash_client::config::errors::ConfigError),

    #[error(transparent)]
    Config(#[from] config::ConfigError),

    #[error("Connection closed by client")]
    ConnectionClosed,

    #[error("Connection timed out")]
    ConnectionTimeout(#[from] Elapsed),

    // #[error(transparent)]
    // Eql(#[from] EqlError),
    #[error(transparent)]
    Io(io::Error),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error(transparent)]
    ZeroKMS(#[from] cipherstash_client::zerokms::Error),
}

#[derive(Error, Debug)]
pub enum EqlError {
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Expected {expected} parameter format codes, received {received}")]
    ParameterFormatCodesMismatch { expected: usize, received: usize },

    #[error("Expected {expected} parameter format codes, received {received}")]
    ParameterResultFormatCodesMismatch { expected: usize, received: usize },

    #[error("Unexpected message length {len} for code {code}")]
    UnexpectedMessageLength { code: u8, len: usize },

    #[error("Unexpected null in string")]
    UnexpectedNull,
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::UnexpectedEof => Error::ConnectionClosed,
            _ => Error::Io(e),
        }
    }
}
