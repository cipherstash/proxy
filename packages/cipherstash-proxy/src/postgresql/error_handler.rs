/// Shared error handling functionality for PostgreSQL protocol components.
///
/// This trait provides consistent error handling between frontend and backend
/// components, ensuring that all errors are properly converted to PostgreSQL
/// ErrorResponse messages and sent to clients in a protocol-compliant manner.
use crate::{
    error::{EncryptError, Error, MappingError},
    postgresql::diagnostics,
};
use pg_proto::DiagnosticResponse;

/// Trait for components that can send PostgreSQL error responses to clients.
///
/// This trait abstracts the common error handling patterns used by both
/// frontend and backend components, providing consistent error conversion
/// and client communication.
pub trait PostgreSqlErrorHandler {
    /// Get the client ID for logging purposes
    fn client_id(&self) -> i32;

    /// Convert various error types into PostgreSQL `DiagnosticResponse` messages.
    ///
    /// # Error Type Mapping
    ///
    /// - `MappingError::InvalidParameter` -> Invalid parameter error
    /// - Other `MappingError` values -> Invalid SQL statement error
    /// - `EncryptError::UnknownColumn` -> Unknown column error
    /// - `EncryptError::CouldNotDecryptDataForKeyset` -> System error
    /// - `EncryptError::UnknownKeysetIdentifier` -> System error
    /// - `Error::ConnectionTimeout` -> Idle session timeout error
    /// - All others -> System error
    ///
    /// # Arguments
    ///
    /// * `err` - The error to be converted to a `DiagnosticResponse`
    fn error_to_response(&self, err: Error) -> DiagnosticResponse {
        match err {
            Error::Mapping(MappingError::InvalidParameter(ref column)) => {
                diagnostics::invalid_parameter(
                    err.to_string(),
                    &column.table_name(),
                    &column.column_name(),
                )
            }
            Error::Mapping(err) => diagnostics::invalid_sql_statement(err.to_string()),
            Error::Encrypt(EncryptError::UnknownColumn {
                ref table,
                ref column,
            }) => diagnostics::unknown_column(err.to_string(), table, column),
            Error::Encrypt(EncryptError::CouldNotDecryptDataForKeyset { .. }) => {
                diagnostics::system_error(err.to_string())
            }
            Error::Encrypt(EncryptError::UnknownKeysetIdentifier { .. }) => {
                diagnostics::system_error(err.to_string())
            }
            Error::ConnectionTimeout { .. } => diagnostics::connection_timeout(err.to_string()),
            _ => diagnostics::system_error(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postgresql::diagnostics::{CODE_IDLE_SESSION_TIMEOUT, CODE_SYSTEM_ERROR};
    use std::time::Duration;

    /// Minimal implementation of PostgreSqlErrorHandler for testing the default method.
    struct TestHandler;

    impl PostgreSqlErrorHandler for TestHandler {
        fn client_id(&self) -> i32 {
            0
        }
    }

    fn field(response: &DiagnosticResponse, code: u8) -> Option<&str> {
        response
            .fields
            .iter()
            .find(|field| field.code == code)
            .and_then(|field| std::str::from_utf8(&field.value).ok())
    }

    #[test]
    fn connection_timeout_maps_to_57p05() {
        let handler = TestHandler;
        let err = Error::ConnectionTimeout {
            duration: Duration::from_millis(5000),
        };
        let response = handler.error_to_response(err);
        assert_eq!(field(&response, b'C'), Some(CODE_IDLE_SESSION_TIMEOUT));
        assert_eq!(
            field(&response, b'M'),
            Some("Connection timed out after 5000 ms")
        );
    }

    #[test]
    fn unknown_error_maps_to_system_error() {
        let handler = TestHandler;
        let err = Error::Unknown;
        let response = handler.error_to_response(err);
        assert_eq!(field(&response, b'C'), Some(CODE_SYSTEM_ERROR));
    }
}
