use cipherstash_client::encryption;
use std::io;
use thiserror::Error;
use tokio::time::error::Elapsed;

use crate::{postgresql::Column, Identifier};

#[derive(Error, Debug)]
pub enum Error {
    #[error("Connection closed after cancel request")]
    CancelRequest,

    #[error(transparent)]
    Client(#[from] cipherstash_client::config::errors::ConfigError),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Context(#[from] ContextError),

    #[error("Connection closed by client")]
    ConnectionClosed,

    #[error("Connection timed out")]
    ConnectionTimeout(#[from] Elapsed),

    #[error("Error creating connection")]
    DatabaseConnection,

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
pub enum ContextError {
    #[error("Portal could not be found in context")]
    UnknownPortal,
}

#[derive(Error, Debug)]
pub enum MappingError {
    #[error("Invalid parameter data for column {} in table {} (OID {})",
    _0.column_name(), _0.table_name(), _0.oid())]
    InvalidParameter(Column),

    #[error(transparent)]
    SqlParse(#[from] sqlparser::parser::ParserError),

    #[error("Encryption of PostgreSQL {name} (OID {oid}) types is not currently supported")]
    UnsupportedParameterType { name: String, oid: u32 },

    #[error("Statement could not be type checked: {0}")]
    StatementCouldNotBeTypeChecked(String),

    #[error("Statement could not be transformed: {0}")]
    StatementCouldNotBeTransformed(String),

    #[error("Could not parse parameter")]
    CouldNotParseParameter,
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error(transparent)]
    Certificate(#[from] rustls_pki_types::pem::Error),

    #[error(transparent)]
    EncryptConfig(#[from] cipherstash_config::errors::ConfigError),

    #[error(transparent)]
    Database(#[from] tokio_postgres::Error),

    #[error("Dataset id is not a valid UUID.")]
    InvalidDatasetId,

    #[error("Server host {name} is not a valid server name")]
    InvalidServerName { name: String },

    #[error("Expected an active Encrypt configuration")]
    MissingActiveEncryptConfig,

    #[error("Missing field {name} from configuration file or environment")]
    MissingParameter { name: String },

    #[error("Expected an Encrypt configuration table")]
    MissingEncryptConfigTable,

    #[error("Missing Transport Layer Security (TLS) certificate")]
    MissingCertificate,

    #[error(transparent)]
    Parse(#[from] serde_json::Error),

    #[error("Database schema could not be loaded")]
    SchemaCouldNotBeLoaded,

    #[error("Client must connect with Transport Layer Security (TLS)")]
    TlsRequired,

    #[error(transparent)]
    FileOrEnvironment(#[from] config::ConfigError),
}

#[derive(Error, Debug)]
pub enum EncryptError {
    #[error(transparent)]
    CiphertextCouldNotBeSerialised(#[from] serde_json::Error),

    #[error("Column {column} in table {table} was not encrypted")]
    ColumnNotEncrypted { table: String, column: String },

    #[error("Expected {expected} columns, received {received}")]
    EncryptedColumnMismatch { expected: usize, received: usize },

    #[error("Decrypted column could not be encoded as the expected type")]
    PlaintextCouldNotBeEncoded,

    #[error(transparent)]
    Pipeline(#[from] encryption::EncryptionError),

    #[error(transparent)]
    PlaintextCouldNotBeDecoded(#[from] cipherstash_client::encryption::TypeParseError),

    #[error("Column {column} in table {table} has no Encrypt configuration")]
    UnknownColumn { table: String, column: String },

    #[error("Table {table} has no Encrypt configuration")]
    UnknownTable { table: String },

    #[error("Unknown Index Term for column {} in table {}", _0.column(), _0.table())]
    UnknownIndexTerm(Identifier),
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("AuthenticationFailed")]
    AuthenticationFailed,

    #[error("Expected {expected} parameter format codes, received {received}")]
    ParameterFormatCodesMismatch { expected: usize, received: usize },

    #[error("Expected {expected} parameter format codes, received {received}")]
    ParameterResultFormatCodesMismatch { expected: usize, received: usize },

    #[error("Expected a {expected} message, received message code {received}")]
    UnexpectedAuthenticationResponse { expected: String, received: i32 },

    #[error("Expected {expected} message code, received {received}")]
    UnexpectedMessageCode { expected: char, received: char },

    #[error("Unexpected message length {len} for code {code}")]
    UnexpectedMessageLength { code: u8, len: usize },

    #[error("Unexpected null in string")]
    UnexpectedNull,

    #[error("Unexpected target {_0}")]
    UnexpectedDescribeTarget(char),

    #[error("Unexpected SASL authentication method {_0}")]
    UnexpectedSaslAuthenticationMethod(String),

    #[error("Unexpected SSLRequest")]
    UnexpectedSSLRequest,

    #[error("Expected a TLS connection")]
    UnexpectedSSLResponse,

    #[error("Unexpected StartupMessage")]
    UnexpectedStartupMessage,

    #[error("Unsupported authentication method {method_code}")]
    UnsupportedAuthentication { method_code: i32 },
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

impl From<cipherstash_client::encryption::TypeParseError> for Error {
    fn from(e: cipherstash_client::encryption::TypeParseError) -> Self {
        Error::Encrypt(e.into())
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
    fn from(_: std::ffi::NulError) -> Self {
        Error::Protocol(ProtocolError::UnexpectedNull)
    }
}

impl From<tokio_postgres::Error> for Error {
    fn from(e: tokio_postgres::Error) -> Self {
        Error::Config(e.into())
    }
}

impl From<std::num::ParseIntError> for Error {
    fn from(_e: std::num::ParseIntError) -> Self {
        MappingError::CouldNotParseParameter.into()
    }
}

impl From<std::num::ParseFloatError> for Error {
    fn from(_e: std::num::ParseFloatError) -> Self {
        MappingError::CouldNotParseParameter.into()
    }
}

impl From<rust_decimal::Error> for Error {
    fn from(_e: rust_decimal::Error) -> Self {
        MappingError::CouldNotParseParameter.into()
    }
}

impl From<chrono::ParseError> for Error {
    fn from(_e: chrono::ParseError) -> Self {
        MappingError::CouldNotParseParameter.into()
    }
}
