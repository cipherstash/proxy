use cipherstash_client::encryption;
use std::io;
use thiserror::Error;
use tokio::time::error::Elapsed;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Connection closed after cancel request")]
    CancelRequest,

    #[error(transparent)]
    Client(#[from] cipherstash_client::config::errors::ConfigError),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("Connection closed by client")]
    ConnectionClosed,

    #[error("Connection timed out")]
    ConnectionTimeout(#[from] Elapsed),

    #[error("Error creating connection after {retries} retries")]
    DatabaseConnection { retries: u32 },

    #[error(transparent)]
    Encrypt(#[from] EncryptError),

    #[error(transparent)]
    Io(io::Error),

    #[error(transparent)]
    Mapping(#[from] MappingError),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error(transparent)]
    Tls(#[from] rustls::Error),

    #[error(transparent)]
    ZeroKMS(#[from] cipherstash_client::zerokms::Error),

    #[error("Unknown error")]
    Unknown,
}

#[derive(Error, Debug)]
pub enum MappingError {
    #[error(transparent)]
    Parse(#[from] sqlparser::parser::ParserError),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error(transparent)]
    Certificate(#[from] rustls_pki_types::pem::Error),

    #[error(transparent)]
    Config(#[from] config::ConfigError),

    #[error(transparent)]
    Dataset(#[from] cipherstash_config::errors::ConfigError),

    #[error(transparent)]
    Database(#[from] tokio_postgres::Error),

    #[error("Expected an active Encrypt configuration")]
    MissingActiveEncryptConfig,

    #[error("Expected an Encrypt configuration table")]
    MissingEncryptConfigTable,

    #[error(transparent)]
    Parse(#[from] serde_json::Error),

    #[error("Server host {name} is not a valid server name")]
    InvalidServerName { name: String },
}

#[derive(Error, Debug)]
pub enum EncryptError {
    #[error("Table {table} has no Encrypt configuration")]
    UnknownTable { table: String },

    #[error("Column {column} in table {table} has no Encrypt configuration")]
    UnknownColumn { table: String, column: String },

    #[error("Column {column} in table {table} was not encrypted")]
    ColumnNotEncrypted { table: String, column: String },

    #[error(transparent)]
    Pipeline(#[from] encryption::EncryptionError),

    #[error(transparent)]
    CiphertextCouldNotBeEncoded(#[from] serde_json::Error),
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

    #[error("Unexpected SSLRequest")]
    UnexpectedSSLRequest,

    #[error("Unexpected StartupMessage")]
    UnexpectedStartupMessage,

    #[error("Expected {expected} message code, received {received}")]
    UnexpectedMessageCode { expected: char, received: char },

    #[error("Expected a TLS connection")]
    UnexpectedSSLResponse,
}

impl From<config::ConfigError> for Error {
    fn from(e: config::ConfigError) -> Self {
        Error::Config(e.into())
    }
}

impl From<cipherstash_config::errors::ConfigError> for Error {
    fn from(e: cipherstash_config::errors::ConfigError) -> Self {
        Error::Config(e.into())
    }
}

impl From<encryption::EncryptionError> for Error {
    fn from(e: encryption::EncryptionError) -> Self {
        Error::Encrypt(e.into())
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::UnexpectedEof => Error::ConnectionClosed,
            _ => Error::Io(e),
        }
    }
}

impl From<rustls_pki_types::pem::Error> for Error {
    fn from(e: rustls_pki_types::pem::Error) -> Self {
        Error::Config(e.into())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Encrypt(e.into())
    }
}

impl From<sqlparser::parser::ParserError> for Error {
    fn from(e: sqlparser::parser::ParserError) -> Self {
        Error::Mapping(e.into())
    }
}

impl From<std::ffi::NulError> for Error {
    fn from(e: std::ffi::NulError) -> Self {
        Error::Protocol(ProtocolError::UnexpectedNull)
    }
}

impl From<tokio_postgres::Error> for Error {
    fn from(e: tokio_postgres::Error) -> Self {
        Error::Config(e.into())
    }
}
