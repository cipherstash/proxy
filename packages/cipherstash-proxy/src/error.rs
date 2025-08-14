use crate::{postgresql::Column, Identifier};
use bytes::BytesMut;
use cipherstash_client::{encryption, schema::ColumnType};
use eql_mapper::{EqlMapperError, EqlTermVariant};
use metrics_exporter_prometheus::BuildError;
use std::{io, time::Duration};
use thiserror::Error;

const ERROR_DOC_BASE_URL: &str = "https://github.com/cipherstash/proxy/blob/main/docs/errors.md";
const ERROR_DOC_CONFIG_URL: &str =
    "https://github.com/cipherstash/proxy/blob/main/docs/how-to/index.md#configuring-proxy";

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

    #[error("Connection timed out after {} ms", duration.as_secs())]
    ConnectionTimeout { duration: Duration },

    #[error("Error creating connection")]
    DatabaseConnection,

    #[error(transparent)]
    Encrypt(#[from] EncryptError),

    #[error(transparent)]
    Io(io::Error),

    #[error(transparent)]
    Mapping(#[from] MappingError),

    #[error(transparent)]
    Prometheus(#[from] BuildError),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error(transparent)]
    Tls(#[from] rustls::Error),

    #[error(transparent)]
    ZeroKMS(#[from] cipherstash_client::zerokms::Error),

    #[error("Unknown error")]
    Unknown,

    #[error(transparent)]
    SendError(#[from] tokio::sync::mpsc::error::SendError<BytesMut>),
}

#[derive(Error, Debug)]
pub enum ContextError {
    #[error("Portal could not be found in context")]
    UnknownPortal,
}

#[derive(Error, Debug)]
pub enum MappingError {
    #[error("Invalid parameter for column '{}' of type '{}' in table '{}' (OID {}). For help visit {}#mapping-invalid-parameter",
    _0.column_name(), _0.cast_type(), _0.table_name(), _0.oid(), ERROR_DOC_BASE_URL)]
    InvalidParameter(Box<Column>),

    #[error(
        "{}. For help visit {}#mapping-invalid-sql-statement",
        _0,
        ERROR_DOC_BASE_URL
    )]
    InvalidSqlStatement(String),

    #[error("Encryption of EQL column {column_type} using strategy {eql_term} is not supported. For help visit {}#mapping-unsupported-parameter-type", ERROR_DOC_BASE_URL)]
    UnsupportedParameterType {
        eql_term: EqlTermVariant,
        column_type: ColumnType,
    },

    #[error("Statement could not be type checked: {}. For help visit {}#mapping-statement-could-not-be-type-checked", _0, ERROR_DOC_BASE_URL)]
    StatementCouldNotBeTypeChecked(String),

    #[error("Statement could not be transformed: {0}")]
    StatementCouldNotBeTransformed(String),

    #[error("Could not parse parameter")]
    CouldNotParseParameter,

    #[error("Statement encountered an internal error. This may be a bug in the statement mapping module of CipherStash Proxy. Please visit {}#mapping-internal-error for more information.", ERROR_DOC_BASE_URL)]
    Internal(String),

    #[error(transparent)]
    EqlMapper(#[from] EqlMapperError),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error(transparent)]
    Certificate(#[from] rustls_pki_types::pem::Error),

    #[error(transparent)]
    EncryptConfig(#[from] cipherstash_client::config::errors::ConfigError),

    #[error(transparent)]
    Database(#[from] tokio_postgres::Error),

    #[error(transparent)]
    FileOrEnvironment(#[from] config::ConfigError),

    #[error("Dataset id is not a valid UUID.")]
    InvalidDatasetId,

    #[error("Server host {name} is not a valid server name")]
    InvalidServerName { name: String },

    #[error("Invalid {name}: {value}")]
    InvalidParameter { name: String, value: String },

    #[error(
        "Invalid Workspace CRN: {crn}. CRN format is `crn:{{region}}.aws:{{workspace_id}}` For help visit {}",
        ERROR_DOC_CONFIG_URL
    )]
    InvalidWorkspaceCrn { crn: String },

    #[error("Missing an active Encrypt configuration")]
    MissingActiveEncryptConfig,

    #[error(
        "Missing {field} from [{key}] configuration. For help visit {}",
        ERROR_DOC_CONFIG_URL
    )]
    MissingFieldForKey { field: String, key: String },

    #[error(
        "Missing {field} from configuration. For help visit {}",
        ERROR_DOC_CONFIG_URL
    )]
    MissingField { field: String },

    #[error(
        "Missing [auth] configuration. Check that workspace_id and client_access_key are defined. For help visit {}",
        ERROR_DOC_CONFIG_URL
    )]
    MissingAuthKey,

    #[error(
        "Missing [encrypt] configuration. Check that client_id, client_key, and default_keyset_id are defined. For help visit {}",
        ERROR_DOC_CONFIG_URL
    )]
    MissingEncryptKey,

    #[error(
        "Missing [database] configuration. Check that username, password, and name are defined. For help visit {}",
        ERROR_DOC_CONFIG_URL
    )]
    MissingDatabaseKey,

    #[error("Expected an Encrypt configuration table")]
    MissingEncryptConfigTable,

    #[error(transparent)]
    Parse(#[from] serde_json::Error),

    #[error("Database schema could not be loaded")]
    SchemaCouldNotBeLoaded,

    #[error("Client must connect with Transport Layer Security (TLS)")]
    TlsRequired,

    #[error(transparent)]
    TlsConfigError(#[from] TlsConfigError),
}

#[derive(Error, Debug)]
pub enum TlsConfigError {
    #[error(
        "Invalid Transport Layer Security (TLS) certificate. For help visit {}#config-missing-or-invalid-tls",
        ERROR_DOC_BASE_URL
    )]
    InvalidCertificate,

    #[error(
        "Invalid Transport Layer Security (TLS) private key. For help visit {}#config-missing-or-invalid-tls",
        ERROR_DOC_BASE_URL
    )]
    InvalidPrivateKey,

    #[error(
        "Missing Transport Layer Security (TLS) certificate at path: {path}. For help visit {}#config-missing-or-invalid-tls",
        ERROR_DOC_BASE_URL
    )]
    MissingCertificate { path: String },

    #[error(
        "Missing Transport Layer Security (TLS) private key at path: {path}. For help visit {}#config-missing-or-invalid-tls",
        ERROR_DOC_BASE_URL
    )]
    MissingPrivateKey { path: String },
}

#[derive(Error, Debug)]
pub enum EncryptError {
    #[error(transparent)]
    CiphertextCouldNotBeSerialised(#[from] serde_json::Error),

    #[error("Encrypted column could not be parsed")]
    ColumnCouldNotBeParsed,

    #[error("Encrypted column is null")]
    ColumnIsNull,

    #[error("Column '{column}' in table '{table}' could not be deserialised. For help visit {}#encrypt-column-could-not-be-deserialised", ERROR_DOC_BASE_URL)]
    ColumnCouldNotBeDeserialised { table: String, column: String },

    #[error("Column '{column}' in table '{table}' could not be encrypted. For help visit {}#encrypt-column-could-not-be-encrypted", ERROR_DOC_BASE_URL)]
    ColumnCouldNotBeEncrypted { table: String, column: String },

    #[error("Column configuration for column '{column}' in table '{table}' does not match the encrypted column. For help visit {}#encrypt-column-config-mismatch", ERROR_DOC_BASE_URL)]
    ColumnConfigurationMismatch { table: String, column: String },

    #[error(
        "Could not decrypt data using keyset '{keyset_id}'. For help visit {}#encrypt-could-not-decrypt-data-for-keyset",
        ERROR_DOC_BASE_URL
    )]
    CouldNotDecryptDataForKeyset { keyset_id: String },

    #[error("InvalidIndexTerm")]
    InvalidIndexTerm,

    #[error(
        "KeysetId `{id}` could not be parsed using `SET CIPHERSTASH.KEYSET_ID`. KeysetId should be a valid UUID. For help visit {}#encrypt-keyset-id-could-not-be-parsed",
        ERROR_DOC_BASE_URL
    )]
    KeysetIdCouldNotBeParsed { id: String },

    #[error(
        "KeysetId could not be set using `SET CIPHERSTASH.KEYSET_ID`. For help visit {}#encrypt-keyset-id-could-not-be-set",
        ERROR_DOC_BASE_URL
    )]
    KeysetIdCouldNotBeSet,

    #[error(
        "A KeysetName could not be set using `SET CIPHERSTASH.KEYSET_NAME`. For help visit {}#encrypt-keyset-name-could-not-be-set",
        ERROR_DOC_BASE_URL
    )]
    KeysetNameCouldNotBeSet,

    /// This should in practice be unreachable
    #[error("Missing encrypt configuration for column type `{plaintext_type}`. For help visit {}#encrypt-missing-encrypt-configuration", ERROR_DOC_BASE_URL)]
    MissingEncryptConfiguration { plaintext_type: String },

    #[error("Decrypted column could not be encoded as the expected type. For help visit {}#encrypt-plaintext-could-not-be-encoded", ERROR_DOC_BASE_URL)]
    PlaintextCouldNotBeEncoded,

    #[error(transparent)]
    Pipeline(#[from] encryption::EncryptionError),

    #[error(transparent)]
    PlaintextCouldNotBeDecoded(#[from] cipherstash_client::encryption::TypeParseError),

    #[error("Missing keyset identifer.")]
    MissingKeysetIdentifier,

    #[error(
        "Cannot SET CIPHERSTASH.KEYSET if a default keyset has been configured. For help visit {}#encrypt-unexpected-set-keyset",
        ERROR_DOC_BASE_URL
    )]
    UnexpectedSetKeyset,

    #[error(
        "Column '{column}' in table '{table}' has no Encrypt configuration. For help visit {}#encrypt-unknown-column",
        ERROR_DOC_BASE_URL
    )]
    UnknownColumn { table: String, column: String },

    #[error(
        "Table '{table}' has no Encrypt configuration. For help visit {}#encrypt-unknown-table",
        ERROR_DOC_BASE_URL
    )]
    UnknownTable { table: String },

    #[error("Unknown Index Term for column '{}' in table '{}'. For help visit {}#encrypt-unknown-index-term", _0.column(), _0.table(), ERROR_DOC_BASE_URL)]
    UnknownIndexTerm(Identifier),
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Database authentication failed: check username and password. For help visit {}#authentication-failed-database", ERROR_DOC_BASE_URL)]
    AuthenticationFailed,

    #[error("Client authentication failed: check username and password. For help visit {}#authentication-failed-client", ERROR_DOC_BASE_URL)]
    ClientAuthenticationFailed,

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

impl From<sqltk::parser::parser::ParserError> for Error {
    fn from(e: sqltk::parser::parser::ParserError) -> Self {
        Error::Mapping(MappingError::InvalidSqlStatement(e.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_error_message() {
        let error = MappingError::Internal("unexpected bug encounterd".to_string());
        let message = error.to_string();

        assert_eq!(format!("Statement encountered an internal error. This may be a bug in the statement mapping module of CipherStash Proxy. Please visit {ERROR_DOC_BASE_URL}#mapping-internal-error for more information."), message);
    }
}
