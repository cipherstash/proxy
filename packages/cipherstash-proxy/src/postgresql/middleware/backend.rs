use super::super::context::Context;
use super::super::data::to_sql;
use super::super::diagnostics::ErrorResponse;
use super::super::error_handler::PostgreSqlErrorHandler;
use super::super::rewrite::row_description::RowDescription;
use super::super::rewrite::UNSPECIFIED_TYPE_OID;
use super::super::Column;
use crate::error::{EncryptError, Error};
use crate::log::{CONTEXT, DEVELOPMENT, MAPPER, PROTOCOL};
use crate::postgresql::context::Portal;
use crate::postgresql::rewrite::data_row::DataRow;
use crate::postgresql::rewrite::param_description::ParamDescription;
use crate::prometheus::{
    DECRYPTED_VALUES_TOTAL, DECRYPTION_DURATION_SECONDS, DECRYPTION_ERROR_TOTAL,
    DECRYPTION_REQUESTS_TOTAL, ROWS_ENCRYPTED_TOTAL, ROWS_PASSTHROUGH_TOTAL, ROWS_TOTAL,
};
use crate::proxy::EncryptionService;
use crate::EqlCiphertext;
use metrics::{counter, histogram};
use pg_proto::{
    AttributedBackendMessages, BackendBatchOutput, BackendMessage, BackendMiddlewareOutput,
    OperationId,
};
use std::time::Instant;
use tracing::{debug, error, info, warn};

/// The PostgreSQL proxy backend that handles server-to-client message processing.
///
/// The Backend intercepts messages from PostgreSQL servers, identifies encrypted data
/// in query results, performs batch decryption, and forwards decrypted results back to
/// PostgreSQL clients. It implements efficient batching strategies to minimize decryption
/// overhead and maintains proper PostgreSQL wire protocol semantics.
///
/// # Message Flow
///
/// ```text
/// Server -> Backend -> Client
///    |         |         |
///    |   [Intercept]     |
///    |   [Buffer Rows]   |
///    |   [Batch Decrypt] |
///    |   [Format Data]   |
///    |         |         |
///    +----> [Forward] ---+
/// ```
///
/// # Key Responsibilities
///
/// - **Result Decryption**: Decrypt encrypted column values in query results
/// - **Batch Processing**: Buffer DataRow messages for efficient batch decryption
/// - **Format Conversion**: Convert decrypted data to appropriate PostgreSQL wire formats
/// - **Protocol Compliance**: Maintain PostgreSQL message ordering and semantics
/// - **Error Handling**: Process and log PostgreSQL error responses
/// - **Metadata Management**: Handle ParameterDescription and RowDescription messages
///
/// # Buffering Strategy
///
/// DataRow messages containing encrypted data are buffered to enable batch decryption:
/// - Buffer fills up to a configurable capacity
/// - Flush occurs on buffer full, session end, or non-DataRow message
/// - Batching reduces encryption API round-trips and improves performance
///
/// # Message Types Handled
///
/// - `DataRow`: Query result rows (buffered for batch decryption)
/// - `CommandComplete`: Indicates end of query execution (triggers flush)
/// - `ErrorResponse`: PostgreSQL error messages (logged and forwarded)
/// - `RowDescription`: Result column metadata (modified for encrypted columns)
/// - `ParameterDescription`: Parameter metadata (modified for encrypted parameters)
/// - `ReadyForQuery`: Session ready state (triggers schema reload if needed)
pub struct Backend<S: EncryptionService> {
    /// Session context with portal and statement metadata
    context: Context<S>,
    /// Buffer for batching DataRow messages before decryption
    buffer: Vec<DataRow>,
    buffer_operation: Option<OperationId>,
    emitted: Vec<BackendMessage>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BackendDisposition {
    #[default]
    Emit,
    Suppress,
}

impl<S: EncryptionService> Backend<S> {
    const RESPONSE_BUFFER_SIZE: usize = 4096;

    /// Creates a new Backend instance.
    ///
    /// # Arguments
    ///
    /// * `client_sender` - Channel sender for sending messages to the client
    /// * `server_reader` - Stream for reading messages from the PostgreSQL server
    /// * `encrypt` - Encryption service for handling column decryption
    /// * `context` - Session context shared with the frontend
    pub fn new(context: Context<S>) -> Self {
        let buffer = Vec::with_capacity(Self::RESPONSE_BUFFER_SIZE);
        Backend {
            context,
            buffer,
            buffer_operation: None,
            emitted: Vec::new(),
        }
    }

    pub async fn intercept(
        &mut self,
        operation: Option<OperationId>,
        message: BackendMessage,
    ) -> Result<BackendMiddlewareOutput, Error> {
        let mut disposition = BackendDisposition::Emit;
        let message = self
            .intercept_backend(operation, &mut disposition, message)
            .await?;
        Ok(if disposition == BackendDisposition::Suppress {
            BackendMiddlewareOutput::Hold
        } else {
            BackendMiddlewareOutput::Forward(message)
        })
    }

    pub async fn flush_held(
        &mut self,
        held: AttributedBackendMessages<'_>,
    ) -> Result<BackendBatchOutput, Error> {
        let operation = held.iter().find_map(|(operation, _)| operation);
        self.flush(operation).await?;
        let messages = std::mem::take(&mut self.emitted);
        if messages.len() != held.iter().len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "decrypted DataRow batch length changed",
            )
            .into());
        }
        Ok(BackendBatchOutput::ReplaceOneToOne(messages))
    }

    async fn intercept_backend(
        &mut self,
        operation: Option<OperationId>,
        disposition: &mut BackendDisposition,
        protocol_message: BackendMessage,
    ) -> Result<BackendMessage, Error> {
        let mut outbound_message = protocol_message.clone();

        if self.context.is_passthrough() {
            debug!(target: DEVELOPMENT,
                client_id = self.context.client_id,
                msg = "Passthrough enabled"
            );
            // A Proxy started against a database with no encrypted columns is
            // initially in passthrough mode. DDL on that connection is how an
            // encrypted schema can first appear, so publish the reload before
            // forwarding ReadyForQuery. This ordering also guarantees that a
            // client opening its next connection after ReadyForQuery observes
            // the newly loaded schema and encrypt configuration.
            if matches!(&protocol_message, BackendMessage::ReadyForQuery(_)) {
                self.context.reload_schema_if_changed().await;
            }

            // CipherStash metadata is operation-keyed even in passthrough mode,
            // and must be released when pg-proto identifies its terminal response.
            match protocol_message {
                BackendMessage::CommandComplete(_)
                | BackendMessage::EmptyQueryResponse
                | BackendMessage::PortalSuspended
                | BackendMessage::ErrorResponse(_) => {
                    if let Some(operation) = operation {
                        let session = self.context.complete_execution(operation);
                        self.context.finish_session(session);
                    }
                }
                _ => {}
            }

            return Ok(outbound_message);
        }

        let keyset_id = self.context.keyset_identifier();
        debug!(target: CONTEXT, client_id = ?self.context.client_id, ?keyset_id);

        match protocol_message {
            BackendMessage::DataRow(row) => {
                // Encrypted DataRows are added to the buffer and we return early
                // Otherwise, continue and write
                if self.data_row_handler(operation, DataRow::from(row)).await? {
                    *disposition = BackendDisposition::Suppress;
                    return Ok(outbound_message);
                }
            }

            // Execute phase is always terminated by the appearance of exactly one of these messages:
            //      CommandComplete, EmptyQueryResponse (if the portal was created from an empty query string), ErrorResponse, or PortalSuspended.
            BackendMessage::CommandComplete(_)
            | BackendMessage::EmptyQueryResponse
            | BackendMessage::PortalSuspended => {
                debug!(target: PROTOCOL, client_id = self.context.client_id, msg = "CommandComplete | EmptyQueryResponse | PortalSuspended");

                match self.flush(operation).await {
                    Ok(_) => (),
                    Err(err) => {
                        warn!(client_id = self.client_id(), error = err.to_string());
                        self.send_error_response(err).await?;
                    }
                }

                if let Some(operation) = operation {
                    let session = self.context.complete_execution(operation);
                    self.context.finish_session(session);
                }
            }
            BackendMessage::ErrorResponse(ref response) => {
                self.error_response_handler(response);

                match self.flush(operation).await {
                    Ok(_) => (),
                    Err(err) => {
                        warn!(client_id = self.client_id(), error = err.to_string());
                        self.send_error_response(err).await?;
                    }
                }

                if let Some(operation) = operation {
                    let session = self.context.complete_execution(operation);
                    self.context.finish_session(session);
                }
            }
            // Describe with Target:Statement
            // Returns a ParameterDescription followed by RowDescription
            // The Describe is complete after the RowDescription
            BackendMessage::ParameterDescription(types) => {
                if let Some(message) = self
                    .parameter_description_handler(operation, ParamDescription::from(types))
                    .await?
                {
                    outbound_message = message;
                }
            }
            // Describe with Target:Statement or Target::Portal
            // Target:Statement returns a ParameterDescription before a RowDescription
            // Target::Portal returns a RowDescription
            // If no rows are returned, NoData is returned instead of a RowDescription
            // Complete the Describe
            BackendMessage::RowDescription(description) => {
                if let Some(message) = self
                    .row_description_handler(operation, RowDescription::from(description))
                    .await?
                {
                    outbound_message = message;
                }
                if let Some(operation) = operation {
                    self.context.complete_describe(operation);
                }
            }
            // Describe with Target:Statement or Target::Portal
            // If the statement returns no rows, NoData is returned instead of a RowDescription
            BackendMessage::NoData => {
                if let Some(operation) = operation {
                    self.context.complete_describe(operation);
                }
            }
            // Reload for SompleQuery flow
            // Reload is potentially triggered by a FrontEnd Sync message.
            // However, the SimpleQuery flow does not use Sync so we check here as well
            BackendMessage::ReadyForQuery(_) => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    msg = "ReadyForQuery"
                );
                self.context.reload_schema_if_changed().await;
            }

            _ => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    msg = "Passthrough",
                    message = ?protocol_message,
                );
            }
        }

        Ok(outbound_message)
    }

    /// Handles PostgreSQL ErrorResponse messages from the server.
    ///
    /// ErrorResponse messages indicate that an error occurred during SQL execution.
    /// This handler logs the errors for debugging and monitoring purposes, then
    /// forwards them to the client unchanged to maintain PostgreSQL compatibility.
    ///
    /// # Error Types
    ///
    /// PostgreSQL can return various types of errors:
    /// - **Syntax Errors**: Malformed SQL statements
    /// - **Permission Errors**: Access denied to tables/columns
    /// - **Constraint Violations**: Primary key, foreign key, etc.
    /// - **Data Errors**: Type mismatches, invalid values
    /// - **System Errors**: Connection issues, resource exhaustion
    ///
    /// # Proxy Error Integration
    ///
    /// Some errors may originate from proxy operations:
    /// - Encryption/decryption failures propagated as database exceptions
    /// - Schema validation errors from EQL mapping
    /// - Key retrieval errors from the encryption service
    ///
    /// These proxy-generated errors are formatted as PostgreSQL-compatible
    /// error responses by the frontend, so they appear as normal database
    /// errors to maintain client compatibility.
    ///
    /// # Logging and Monitoring
    ///
    /// All errors are logged at ERROR level for debugging and recorded in
    /// monitoring systems. This helps track both application-level issues
    /// and proxy-specific problems.
    ///
    /// # Returns
    ///
    /// Always returns `Some(bytes)` containing the original error response
    /// to forward to the client unchanged.
    fn error_response_handler(&mut self, response: &pg_proto::DiagnosticResponse) {
        let error_response = ErrorResponse::from(response);
        error!(msg = "PostgreSQL Error", error = ?error_response);
        info!(msg = "PostgreSQL Errors originate in the database");
    }

    ///
    /// DataRows are buffered so that Decryption can be batched
    /// Decryption will occur
    ///  - on direct call to flush()
    ///  - when the buffer is full
    ///  - when any other message type is written
    ///
    async fn buffer(&mut self, operation: OperationId, data_row: DataRow) -> Result<(), Error> {
        if self
            .buffer_operation
            .is_some_and(|current| current != operation)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "DataRow batch spans operations",
            )
            .into());
        }
        self.buffer_operation = Some(operation);
        self.buffer.push(data_row);
        Ok(())
    }

    ///
    /// Write a message to the client
    /// Flushes all messages in the buffer before writing the message
    ///
    pub async fn write_with_flush(&mut self, message: BackendMessage) -> Result<(), Error> {
        debug!(target: DEVELOPMENT, client_id = self.context.client_id, msg = "Write");

        match self.flush(None).await {
            Ok(_) => (),
            Err(err) => {
                warn!(client_id = self.client_id(), error = err.to_string());
                self.send_error_response(err).await?;
            }
        }

        self.write(message).await?;
        Ok(())
    }

    ///
    /// Write a message to the client
    ///
    pub async fn write(&mut self, message: BackendMessage) -> Result<(), Error> {
        self.emitted.push(message);

        Ok(())
    }

    /// Flushes all buffered DataRow messages by performing batch decryption.
    ///
    /// This is the core decryption logic that processes buffered DataRow messages,
    /// extracts encrypted column values, performs batch decryption, and sends the
    /// decrypted results back to the client in the proper PostgreSQL wire format.
    ///
    /// # Process Overview
    ///
    /// 1. **Portal Validation**: Check if current portal requires decryption
    /// 2. **Data Extraction**: Extract encrypted values from buffered DataRows
    /// 3. **Batch Decryption**: Send all encrypted values to decryption service
    /// 4. **Format Conversion**: Convert decrypted plaintext to PostgreSQL wire format
    /// 5. **Result Assembly**: Reconstruct DataRows with decrypted values
    /// 6. **Client Delivery**: Send decrypted DataRows to client
    ///
    /// # Portal-Based Processing
    ///
    /// Decryption behavior is determined by the portal associated with the current execution:
    /// - **Encrypted Portal**: Contains column metadata for decryption
    /// - **Passthrough Portal**: No decryption needed, should not have buffered data
    ///
    /// # Batch Decryption Benefits
    ///
    /// - **Performance**: Single API call for multiple encrypted values
    /// - **Efficiency**: Reduces network round-trips to encryption service
    /// - **Consistency**: All values decrypted with same keyset ID
    ///
    /// # Format Code Handling
    ///
    /// Result columns can be formatted as text or binary based on format codes
    /// specified in the original Bind message. Decrypted values are properly
    /// encoded according to these format specifications.
    ///
    /// # Error Handling
    ///
    /// Decryption errors (including key retrieval failures) are converted to
    /// appropriate error responses and recorded in metrics. The error mapping
    /// implemented in the encryption service ensures proper keyset ID context
    /// is preserved in error messages.
    async fn flush(&mut self, operation: Option<OperationId>) -> Result<(), Error> {
        if self.buffer.is_empty() {
            debug!(target: MAPPER, client_id = self.context.client_id, msg = "Empty buffer");
        }

        let operation = operation.or(self.buffer_operation);
        let portal =
            operation.and_then(|operation| self.context.get_portal_from_execute(operation));
        let portal = match portal.as_deref() {
            Some(Portal::Encrypted { .. }) => portal.unwrap(),
            _ => {
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Passthrough portal");
                if !self.buffer.is_empty() {
                    error!(
                        client_id = self.context.client_id,
                        msg = "Buffer is not empty"
                    );
                }
                return Ok(());
            }
        };

        let mut rows = std::mem::take(&mut self.buffer);
        self.buffer_operation = None;
        debug!(target: DEVELOPMENT, client_id = self.context.client_id, rows = rows.len());

        let result_column_count = match rows.first() {
            Some(row) => row.column_count(),
            None => return Ok(()),
        };

        // Result Column Format Codes are passed with the Bind message
        // Bind is turned into a Portal
        // We pull the format codes from the portal
        // If no portal, assume Text for all columns
        let result_column_format_codes = portal.format_codes(result_column_count);

        let projection_columns = portal.projection_columns();

        // Each row is converted into Vec<Option<CipherText>>
        let ciphertexts: Vec<Option<EqlCiphertext>> = rows
            .iter_mut()
            .flat_map(|row| row.as_ciphertext(projection_columns))
            .collect::<Vec<_>>();

        let start = Instant::now();

        self.check_column_config(projection_columns, &ciphertexts)?;

        let keyset_id = self.context.keyset_identifier();

        debug!(target: CONTEXT,
            client_id = self.context.client_id,
            ?keyset_id,
        );

        // Decrypt CipherText -> Plaintext
        let plaintexts = self.context.decrypt(ciphertexts).await.inspect_err(|_| {
            counter!(DECRYPTION_ERROR_TOTAL).increment(1);
        })?;

        let duration = Instant::now().duration_since(start);

        // Always record for slow-statement diagnostics
        if let Some(operation) = operation {
            self.context
                .add_decrypt_duration_for_execute(operation, duration);
        }

        // Prometheus metrics remain gated
        if self.context.prometheus_enabled() {
            let decrypted_count =
                plaintexts
                    .iter()
                    .fold(0, |acc, o| if o.is_some() { acc + 1 } else { acc });

            counter!(DECRYPTION_REQUESTS_TOTAL).increment(1);
            counter!(DECRYPTED_VALUES_TOTAL).increment(decrypted_count);
            histogram!(DECRYPTION_DURATION_SECONDS).record(duration);
        }

        // Chunk rows into sets of columns
        let rows = plaintexts.chunks(result_column_count).zip(rows);

        // Stitch Plaintext back into Rows encoded with the appropriate Format Code
        // Each chunk is written to the client
        for (chunk, mut row) in rows {
            let data = chunk
                .iter()
                .zip(result_column_format_codes.iter())
                .map(|(plaintext, format_code)| match plaintext {
                    Some(plaintext) => to_sql(plaintext, format_code),
                    None => Ok(None),
                })
                .collect::<Result<Vec<_>, _>>()?;

            row.rewrite(&data)?;

            self.write(BackendMessage::from(row)).await?;
        }

        Ok(())
    }

    fn check_column_config(
        &mut self,
        projection_columns: &[Option<Column>],
        ciphertexts: &[Option<EqlCiphertext>],
    ) -> Result<(), Error> {
        for (col, ct) in projection_columns.iter().zip(ciphertexts) {
            match (col, ct) {
                (Some(col), Some(ct)) => {
                    if &col.identifier != ct.identifier() {
                        return Err(EncryptError::ColumnConfigurationMismatch {
                            table: col.identifier.table.to_owned(),
                            column: col.identifier.column.to_owned(),
                        }
                        .into());
                    }
                }
                // configured column with NULL ciphertext
                (Some(_), None) => {}
                // unconfigured column *should* have no ciphertext,
                (None, None) => {}
                // ciphertext with no column configuration is bad
                (None, Some(ct)) => {
                    return Err(EncryptError::ColumnConfigurationMismatch {
                        table: ct.identifier().table.to_owned(),
                        column: ct.identifier().column.to_owned(),
                    }
                    .into());
                }
            }
        }
        Ok(())
    }

    async fn parameter_description_handler(
        &self,
        operation: Option<OperationId>,
        mut description: ParamDescription,
    ) -> Result<Option<BackendMessage>, Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, ParamDescription = ?description);

        if let Some(statement) =
            operation.and_then(|operation| self.context.get_statement_from_describe(operation))
        {
            // Describe the params the CLIENT wrote, not the ones PostgreSQL was
            // sent. A rewrite may have fused or dropped params, in which case
            // the server's description is both shorter than and shifted from
            // what the client needs in order to bind.
            let param_types = statement
                .param_columns
                .iter()
                .enumerate()
                .map(|(idx, col)| match col {
                    Some(col) => {
                        debug!(target: MAPPER, client_id = self.context.client_id, ColumnConfig = ?col);
                        col.postgres_type.oid() as i32
                    }
                    // A native param is never fused, so it reaches PostgreSQL
                    // as some output param; take the type the server inferred
                    // for it.
                    None => statement
                        .output_params
                        .iter()
                        .position(|output| output.source.primary_input() == idx)
                        .and_then(|output_idx| description.types.get(output_idx).copied())
                        .unwrap_or(UNSPECIFIED_TYPE_OID),
                })
                .collect::<Vec<_>>();

            debug!(target: MAPPER, client_id = self.context.client_id, param_types = ?param_types);

            description.set_types(param_types);
        }

        if description.requires_rewrite() {
            let message = BackendMessage::from(description);
            debug!(target: MAPPER, client_id = self.context.client_id, msg = "Rewrite ParamDescription", ?message);
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    ///
    ///
    /// RowDescription message handler
    ///
    ///
    ///
    ///
    async fn row_description_handler(
        &mut self,
        operation: Option<OperationId>,
        mut description: RowDescription,
    ) -> Result<Option<BackendMessage>, Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, RowDescription = ?description);

        if let Some(statement) =
            operation.and_then(|operation| self.context.get_statement_for_operation(operation))
        {
            let projection_types = statement
                .projection_columns
                .iter()
                .map(|col| col.as_ref().map(|col| col.postgres_type.clone()))
                .collect::<Vec<_>>();

            debug!(target: MAPPER, client_id = self.context.client_id, projection_types = ?projection_types);

            description.map_types(&projection_types);
        }

        if description.requires_rewrite() {
            let message = BackendMessage::from(description);
            debug!(target: MAPPER, client_id = self.context.client_id, msg = "Rewrite RowDescription", ?message);
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    /// Handles PostgreSQL DataRow messages containing query result data.
    ///
    /// DataRow messages contain the actual row data returned by SELECT queries.
    /// This handler determines whether rows contain encrypted data that needs
    /// decryption, and either buffers them for batch processing or passes them
    /// through unchanged.
    ///
    /// # Processing Decision
    ///
    /// The handler examines the portal associated with the current execution:
    /// - **Encrypted Portal**: Rows may contain encrypted data, buffer for decryption
    /// - **Passthrough Portal**: Rows contain no encrypted data, forward immediately
    /// - **No Portal**: No execution context, forward immediately
    ///
    /// # Buffering Strategy
    ///
    /// Encrypted rows are added to an internal buffer rather than being processed
    /// immediately. This enables:
    /// - Batch decryption of multiple encrypted values
    /// - Improved performance through reduced API calls
    /// - Better error handling for decryption operations
    ///
    /// The buffer is automatically flushed when it reaches capacity or when
    /// the query execution completes.
    ///
    /// # Return Value
    ///
    /// Returns `Ok(true)` if the row was buffered (caller should not forward),
    /// or `Ok(false)` if the row should be forwarded unchanged by the caller.
    ///
    /// # Metrics
    ///
    /// Records metrics for both encrypted and passthrough row processing to
    /// track proxy performance and encryption usage patterns.
    async fn data_row_handler(
        &mut self,
        operation: Option<OperationId>,
        data_row: DataRow,
    ) -> Result<bool, Error> {
        counter!(ROWS_TOTAL).increment(1);
        match operation
            .and_then(|operation| self.context.get_portal_from_execute(operation))
            .as_deref()
        {
            Some(Portal::Encrypted { .. }) => {
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Encrypted");

                self.buffer(
                    operation.expect("encrypted DataRow has an operation"),
                    data_row,
                )
                .await?;

                counter!(ROWS_ENCRYPTED_TOTAL).increment(1);
                Ok(true)
            }
            _ => {
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Passthrough");
                counter!(ROWS_PASSTHROUGH_TOTAL).increment(1);
                Ok(false)
            }
        }
    }
}

/// Implementation of PostgreSQL error handling for the Backend component.
impl<S: EncryptionService> PostgreSqlErrorHandler for Backend<S> {
    fn client_id(&self) -> i32 {
        self.context.client_id
    }
}

impl<S: EncryptionService> Backend<S> {
    async fn send_error_response(&mut self, err: Error) -> Result<(), Error> {
        let error_response = self.error_to_response(err);
        // Ensure any buffered data is cleared before sending error
        self.buffer.clear();
        self.buffer_operation = None;

        let message = error_response.into_backend_message();

        debug!(
            target: "PROTOCOL",
            client_id = self.context.client_id,
            msg = "backend_send_error_response",
            ?message,
        );

        self.emitted.push(message);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TandemConfig;
    use crate::postgresql::context::KeysetIdentifier;
    use crate::proxy::{EncryptConfig, EncryptionService};
    use eql_mapper::Schema;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    struct TestService {}

    #[async_trait::async_trait]
    impl EncryptionService for TestService {
        async fn encrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
            _columns: &[Option<Column>],
        ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
            Ok(vec![])
        }

        async fn decrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn passthrough_reloads_changed_schema_before_ready_for_query() {
        let config = Arc::new(TandemConfig::for_testing());
        let encrypt_config = Arc::new(EncryptConfig::default());
        let schema = Arc::new(Schema::new("public"));
        let (reload_sender, mut reload_receiver) = mpsc::unbounded_channel();
        let context = Context::new(
            1,
            config,
            encrypt_config,
            schema,
            TestService {},
            reload_sender,
        );
        context.set_schema_changed();

        let reload_task = tokio::spawn(async move {
            let Some(crate::proxy::ReloadCommand::DatabaseSchema(responder)) =
                reload_receiver.recv().await
            else {
                panic!("expected a database schema reload command");
            };
            responder.send(true).expect("reload receiver must be open");
        });

        let mut backend = Backend::new(context);
        let ready = BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle);
        let output = backend.intercept(None, ready.clone()).await.unwrap();
        reload_task.await.unwrap();
        assert!(matches!(
            output,
            BackendMiddlewareOutput::Forward(message) if message == ready
        ));
        assert!(!backend.context.take_schema_changed());
    }

}
