/// Shared error handling functionality for PostgreSQL protocol components.
///
/// This trait provides consistent error handling between frontend and backend
/// components, ensuring that all errors are properly converted to PostgreSQL
/// ErrorResponse messages and sent to clients in a protocol-compliant manner.
use crate::{
    connect::Sender,
    error::{EncryptError, Error, MappingError},
    postgresql::messages::error_response::ErrorResponse,
};

/// Trait for components that can send PostgreSQL error responses to clients.
///
/// This trait abstracts the common error handling patterns used by both
/// frontend and backend components, providing consistent error conversion
/// and client communication.
pub trait PostgreSqlErrorHandler {
    /// Get the client sender for this component
    fn client_sender(&mut self) -> &mut Sender;

    /// Get the client ID for logging purposes
    fn client_id(&self) -> i32;

    /// Convert various error types into appropriate PostgreSQL ErrorResponse messages.
    ///
    /// # Error Type Mapping
    ///
    /// - `MappingError` -> InvalidSqlStatement error
    /// - `EncryptError::UnknownColumn` -> Unknown column error
    /// - `EncryptError::CouldNotRetrieveKey` -> Key retrieval error
    /// - All others -> System error
    ///
    /// # Arguments
    ///
    /// * `err` - The error to be converted to a PostgreSQL ErrorResponse
    fn error_to_response(&self, err: Error) -> ErrorResponse {
        match err {
            Error::Mapping(MappingError::InvalidParameter(ref column)) => {
                ErrorResponse::invalid_parameter(
                    err.to_string(),
                    &column.table_name(),
                    &column.column_name(),
                )
            }
            Error::Mapping(err) => ErrorResponse::invalid_sql_statement(err.to_string()),
            Error::Encrypt(EncryptError::UnknownColumn {
                ref table,
                ref column,
            }) => ErrorResponse::unknown_column(err.to_string(), table, column),
            Error::Encrypt(EncryptError::CouldNotDecryptDataForKeyset { .. }) => {
                ErrorResponse::system_error(err.to_string())
            }
            _ => ErrorResponse::system_error(err.to_string()),
        }
    }

    /// Send an ErrorResponse message to the client.
    ///
    /// Converts the error to a PostgreSQL ErrorResponse and sends it
    /// to the client via the component's sender channel.
    ///
    /// # Arguments
    ///
    /// * `error_response` - The ErrorResponse to send to the client
    fn send_error_response(&mut self, err: Error) -> Result<(), Error>;
    // fn send_error_response(&mut self, err: Error) -> Result<(), Error> {
    //     let error_response = self.error_to_response(err);

    //     let message = BytesMut::try_from(error_response)?;

    //     debug!(
    //         target: "PROTOCOL",
    //         client_id = self.client_id(),
    //         msg = "send_error_response",
    //         ?message,
    //     );

    //     self.client_sender().send(message)?;
    //     Ok(())
    // }
}
