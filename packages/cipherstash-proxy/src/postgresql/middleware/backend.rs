use super::super::context::{Context, ExecutionOutcome};
use super::super::data::to_sql;
use super::super::error_handler::PostgreSqlErrorHandler;
use super::super::rewrite::UNSPECIFIED_TYPE_OID;
use super::super::Column;
use crate::error::{EncryptError, Error, ProtocolError};
use crate::log::{CONTEXT, DEVELOPMENT, MAPPER, PROTOCOL};
use crate::postgresql::context::Portal;
use crate::postgresql::rewrite::data_row;
use crate::postgresql::OperationId;
use crate::prometheus::{
    DECRYPTED_VALUES_TOTAL, DECRYPTION_DURATION_SECONDS, DECRYPTION_ERROR_TOTAL,
    DECRYPTION_REQUESTS_TOTAL, ROWS_ENCRYPTED_TOTAL, ROWS_PASSTHROUGH_TOTAL, ROWS_TOTAL,
};
use crate::proxy::EncryptionService;
use crate::EqlCiphertext;
use metrics::{counter, histogram};
use pg_proto::{BackendMessage, BackendMiddlewareOutput, DataRow, DiagnosticResponse};
use std::time::Instant;
use tracing::{debug, error, info};

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
/// - Flush occurs on buffer full, connection end, or non-DataRow message
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
    encrypted_rows: Vec<DataRow>,
    encrypted_rows_operation: Option<OperationId>,
    encrypted_rows_bytes: usize,
    discard_execution: bool,
}

const MAX_ENCRYPTED_ROWS: usize = 4096;
const MAX_ENCRYPTED_ROW_BYTES: usize = 64 * 1024 * 1024;

fn execution_outcome(message: &BackendMessage) -> Option<ExecutionOutcome> {
    match message {
        BackendMessage::CommandComplete(_) | BackendMessage::EmptyQueryResponse => {
            Some(ExecutionOutcome::Completed)
        }
        BackendMessage::PortalSuspended => Some(ExecutionOutcome::Suspended),
        _ => None,
    }
}

impl<S: EncryptionService> Backend<S> {
    fn finish_execution(
        &self,
        operation: Option<OperationId>,
        outcome: ExecutionOutcome,
    ) -> Result<Option<DiagnosticResponse>, Error> {
        let Some(operation) = operation else {
            return Ok(None);
        };
        self.context.finish_execution(operation, outcome)
    }

    fn error_response(&self, err: Error) -> BackendMessage {
        BackendMessage::ErrorResponse(self.error_to_response(err))
    }

    fn complete_describe(&self, operation: OperationId) -> Result<(), Error> {
        self.context.complete_describe(operation)
    }

    fn decryption_failure(&mut self, err: Error) -> BackendMiddlewareOutput {
        self.encrypted_rows.clear();
        self.encrypted_rows_operation = None;
        self.encrypted_rows_bytes = 0;
        self.discard_execution = true;
        BackendMiddlewareOutput::Expand(vec![self.error_response(err)])
    }

    /// Creates a new Backend instance.
    ///
    /// # Arguments
    ///
    /// * `client_sender` - Channel sender for sending messages to the client
    /// * `server_reader` - Stream for reading messages from the PostgreSQL server
    /// * `encrypt` - Encryption service for handling column decryption
    /// * `context` - Session context shared with the frontend
    pub fn new(context: Context<S>) -> Self {
        Backend {
            context,
            encrypted_rows: Vec::new(),
            encrypted_rows_operation: None,
            encrypted_rows_bytes: 0,
            discard_execution: false,
        }
    }

    pub async fn intercept(
        &mut self,
        operation: Option<OperationId>,
        message: BackendMessage,
    ) -> Result<BackendMiddlewareOutput, Error> {
        self.intercept_backend(operation, message).await
    }

    async fn intercept_backend(
        &mut self,
        operation: Option<OperationId>,
        protocol_message: BackendMessage,
    ) -> Result<BackendMiddlewareOutput, Error> {
        let mut outbound_message = protocol_message.clone();

        if matches!(
            protocol_message,
            BackendMessage::ParseComplete | BackendMessage::BindComplete
        ) {
            if let Some(operation) = operation {
                self.context.complete_non_execution(operation)?;
            }
        }

        if self.context.is_passthrough() {
            match &protocol_message {
                BackendMessage::CommandComplete(_)
                | BackendMessage::EmptyQueryResponse
                | BackendMessage::PortalSuspended => {
                    self.context.report_schema_execution_succeeded();
                }
                BackendMessage::ErrorResponse(_) => {
                    self.context.report_schema_execution_failed();
                }
                _ => {}
            }
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
            if let BackendMessage::ReadyForQuery(status) = &protocol_message {
                self.handle_ready_for_query(operation, *status).await?;
            }

            // CipherStash metadata is operation-keyed even in passthrough mode,
            // and must be released when pg-proto identifies its terminal response.
            if let Some(outcome) = execution_outcome(&protocol_message) {
                if let Some(response) = self.finish_execution(operation, outcome)? {
                    outbound_message = BackendMessage::ErrorResponse(response);
                }
            } else if matches!(protocol_message, BackendMessage::ErrorResponse(_))
                && operation.is_some()
            {
                if let Some(response) =
                    self.finish_execution(operation, ExecutionOutcome::Failed)?
                {
                    outbound_message = BackendMessage::ErrorResponse(response);
                }
            } else {
                match protocol_message {
                    BackendMessage::RowDescription(_) | BackendMessage::NoData => {
                        if let Some(operation) = operation {
                            self.complete_describe(operation)?;
                        }
                    }
                    BackendMessage::ReadyForQuery(_) => {}
                    _ => {}
                }
            }

            return Ok(BackendMiddlewareOutput::Forward(outbound_message));
        }

        if self.discard_execution {
            match protocol_message {
                BackendMessage::CommandComplete(_)
                | BackendMessage::EmptyQueryResponse
                | BackendMessage::PortalSuspended => {
                    self.context.report_schema_execution_succeeded();
                    if let Some(operation) = operation {
                        self.context.discard_operation(operation)?;
                    }
                }
                BackendMessage::ErrorResponse(_) => {
                    self.context.report_schema_execution_failed();
                    if let Some(operation) = operation {
                        self.context.discard_operation(operation)?;
                    }
                }
                BackendMessage::ReadyForQuery(status) => {
                    self.discard_execution = false;
                    self.handle_ready_for_query(operation, status).await?;
                    return Ok(BackendMiddlewareOutput::Forward(
                        BackendMessage::ReadyForQuery(status),
                    ));
                }
                _ => {}
            }
            return Ok(BackendMiddlewareOutput::Suppress(protocol_message));
        }

        let mut prefix = if !matches!(protocol_message, BackendMessage::DataRow(_))
            && !self.encrypted_rows.is_empty()
        {
            let encrypted_rows_operation = self.encrypted_rows_operation;
            match self.flush_encrypted_rows().await {
                Ok(messages) => messages,
                Err(err) => {
                    self.discard_execution = true;
                    self.finish_execution(encrypted_rows_operation, ExecutionOutcome::Failed)?;
                    return Ok(self.decryption_failure(err));
                }
            }
        } else {
            Vec::new()
        };

        match &protocol_message {
            BackendMessage::CommandComplete(_)
            | BackendMessage::EmptyQueryResponse
            | BackendMessage::PortalSuspended => {
                self.context.report_schema_execution_succeeded();
            }
            BackendMessage::ErrorResponse(_) => {
                self.context.report_schema_execution_failed();
            }
            _ => {}
        }

        let keyset_id = self.context.keyset_identifier();
        debug!(target: CONTEXT, client_id = ?self.context.client_id, ?keyset_id);

        match protocol_message {
            BackendMessage::DataRow(row) => {
                // Encrypted DataRows are added to the buffer and we return early
                // Otherwise, continue and write
                if self.data_row_handler(operation).await? {
                    let Some(operation) = operation else {
                        return Ok(self.decryption_failure(
                            ProtocolError::HeldDataRowMissingOperation.into(),
                        ));
                    };
                    if self
                        .encrypted_rows_operation
                        .replace(operation)
                        .is_some_and(|current| current != operation)
                    {
                        return Ok(self.decryption_failure(
                            ProtocolError::HeldDataRowOperationMismatch.into(),
                        ));
                    }
                    self.encrypted_rows_bytes += row
                        .columns
                        .iter()
                        .flatten()
                        .map(bytes::Bytes::len)
                        .sum::<usize>();
                    self.encrypted_rows.push(row.clone());
                    if self.encrypted_rows.len() < MAX_ENCRYPTED_ROWS
                        && self.encrypted_rows_bytes < MAX_ENCRYPTED_ROW_BYTES
                    {
                        return Ok(BackendMiddlewareOutput::Suppress(BackendMessage::DataRow(
                            row,
                        )));
                    }
                    return match self.flush_encrypted_rows().await {
                        Ok(messages) => Ok(BackendMiddlewareOutput::Expand(messages)),
                        Err(err) => {
                            self.finish_execution(Some(operation), ExecutionOutcome::Failed)?;
                            Ok(self.decryption_failure(err))
                        }
                    };
                }
            }

            // Execute phase is always terminated by the appearance of exactly one of these messages:
            //      CommandComplete, EmptyQueryResponse (if the portal was created from an empty query string), or PortalSuspended.
            terminal @ (BackendMessage::CommandComplete(_)
            | BackendMessage::EmptyQueryResponse
            | BackendMessage::PortalSuspended) => {
                let outcome = execution_outcome(&terminal).unwrap();
                debug!(target: PROTOCOL, client_id = self.context.client_id, ?outcome, msg = "Execute outcome");
                if let Some(response) = self.finish_execution(operation, outcome)? {
                    outbound_message = BackendMessage::ErrorResponse(response);
                }
            }
            BackendMessage::ErrorResponse(response) => {
                self.error_response_handler(&response);
                if operation.is_some() {
                    if let Some(response) =
                        self.finish_execution(operation, ExecutionOutcome::Failed)?
                    {
                        outbound_message = BackendMessage::ErrorResponse(response);
                    }
                }
            }
            // Describe with Target:Statement
            // Returns a ParameterDescription followed by RowDescription
            // The Describe is complete after the RowDescription
            BackendMessage::ParameterDescription(types) => {
                if let Some(message) = self.parameter_description_handler(operation, types).await? {
                    outbound_message = message;
                }
            }
            // Describe with Target:Statement or Target::Portal
            // Target:Statement returns a ParameterDescription before a RowDescription
            // Target::Portal returns a RowDescription
            // If no rows are returned, NoData is returned instead of a RowDescription
            // Complete the Describe
            BackendMessage::RowDescription(description) => {
                if let Some(message) = self.row_description_handler(operation, description).await? {
                    outbound_message = message;
                }
                if let Some(operation) = operation {
                    self.complete_describe(operation)?;
                }
            }
            // Describe with Target:Statement or Target::Portal
            // If the statement returns no rows, NoData is returned instead of a RowDescription
            BackendMessage::NoData => {
                if let Some(operation) = operation {
                    self.complete_describe(operation)?;
                }
            }
            // Reload for SompleQuery flow
            // Reload is potentially triggered by a FrontEnd Sync message.
            // However, the SimpleQuery flow does not use Sync so we check here as well
            BackendMessage::ReadyForQuery(status) => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    msg = "ReadyForQuery"
                );
                self.handle_ready_for_query(operation, status).await?;
            }

            _ => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    msg = "Passthrough",
                    message = ?protocol_message,
                );
            }
        }

        if prefix.is_empty() {
            Ok(BackendMiddlewareOutput::Forward(outbound_message))
        } else {
            prefix.push(outbound_message);
            Ok(BackendMiddlewareOutput::Expand(prefix))
        }
    }

    /// Publishes committed DDL before exposing idle readiness, then updates
    /// connection-local transaction state from the same authoritative boundary.
    async fn handle_ready_for_query(
        &mut self,
        operation: Option<OperationId>,
        status: pg_proto::TransactionStatus,
    ) -> Result<(), Error> {
        self.context.ready_for_query(status, operation)?;
        if status == pg_proto::TransactionStatus::Idle {
            self.context.publish_schema_if_changed().await?;
        }
        let schema_status = match status {
            pg_proto::TransactionStatus::Idle => crate::proxy::schema::TransactionStatus::Idle,
            pg_proto::TransactionStatus::InTransaction => {
                crate::proxy::schema::TransactionStatus::InTransaction
            }
            pg_proto::TransactionStatus::FailedTransaction => {
                crate::proxy::schema::TransactionStatus::FailedTransaction
            }
        };
        self.context.schema_ready_for_query(schema_status);
        Ok(())
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
        error!(msg = "PostgreSQL Error", fields = ?response.fields);
        info!(msg = "PostgreSQL Errors originate in the database");
    }

    ///
    /// DataRows are buffered so that Decryption can be batched
    /// Decryption will occur
    ///  - on direct call to flush()
    ///  - when the buffer is full
    ///  - when any other message type is written
    ///
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
    async fn flush_encrypted_rows(&mut self) -> Result<Vec<BackendMessage>, Error> {
        let operation = self.encrypted_rows_operation.take();
        let mut rows = std::mem::take(&mut self.encrypted_rows);
        self.encrypted_rows_bytes = 0;

        let portal = operation
            .map(|operation| self.context.get_portal_from_execute(operation))
            .transpose()?
            .flatten();
        let portal = match portal.as_deref() {
            Some(Portal::Encrypted { .. }) => portal.unwrap(),
            _ => {
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Passthrough portal");
                return Err(ProtocolError::HeldDataRowsNotEncrypted.into());
            }
        };
        debug!(target: DEVELOPMENT, client_id = self.context.client_id, rows = rows.len());

        let result_column_count = match rows.first() {
            Some(row) => row.columns.len(),
            None => return Ok(Vec::new()),
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
            .flat_map(|row| data_row::as_ciphertext(row, projection_columns))
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
                .add_decrypt_duration_for_execute(operation, duration)?;
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
        let mut messages = Vec::with_capacity(rows.len());
        for (chunk, mut row) in rows {
            let data = chunk
                .iter()
                .zip(result_column_format_codes.iter())
                .map(|(plaintext, format_code)| match plaintext {
                    Some(plaintext) => to_sql(plaintext, format_code),
                    None => Ok(None),
                })
                .collect::<Result<Vec<_>, _>>()?;

            data_row::rewrite(&mut row, &data)?;

            messages.push(BackendMessage::DataRow(row));
        }
        Ok(messages)
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
        description: Vec<u32>,
    ) -> Result<Option<BackendMessage>, Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, ParamDescription = ?description);

        let statement = operation
            .map(|operation| self.context.get_statement_from_describe(operation))
            .transpose()?
            .flatten();
        if let Some(statement) = statement {
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
                        .and_then(|output_idx| description.get(output_idx).copied())
                        .map(|oid| oid as i32)
                        .unwrap_or(UNSPECIFIED_TYPE_OID),
                })
                .collect::<Vec<_>>();

            debug!(target: MAPPER, client_id = self.context.client_id, param_types = ?param_types);

            let rewritten = param_types
                .into_iter()
                .map(|oid| oid as u32)
                .collect::<Vec<_>>();
            if rewritten != description {
                let message = BackendMessage::ParameterDescription(rewritten);
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Rewrite ParamDescription", ?message);
                return Ok(Some(message));
            }
        }
        Ok(None)
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
        mut description: pg_proto::RowDescription,
    ) -> Result<Option<BackendMessage>, Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, RowDescription = ?description);

        let statement = operation
            .map(|operation| self.context.get_statement_for_operation(operation))
            .transpose()?
            .flatten();
        if let Some(statement) = statement {
            let projection_types = statement
                .projection_columns
                .iter()
                .map(|col| col.as_ref().map(|col| col.postgres_type.clone()))
                .collect::<Vec<_>>();

            debug!(target: MAPPER, client_id = self.context.client_id, projection_types = ?projection_types);

            let mut rewritten = false;
            for (field, postgres_type) in description.fields.iter_mut().zip(projection_types) {
                if let Some(postgres_type) = postgres_type {
                    let oid = postgres_type.oid();
                    rewritten |= field.type_oid != oid;
                    field.type_oid = oid;
                }
            }
            if rewritten {
                let message = BackendMessage::RowDescription(description);
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Rewrite RowDescription", ?message);
                return Ok(Some(message));
            }
        }
        Ok(None)
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
    async fn data_row_handler(&mut self, operation: Option<OperationId>) -> Result<bool, Error> {
        counter!(ROWS_TOTAL).increment(1);
        let portal = operation
            .map(|operation| self.context.get_portal_from_execute(operation))
            .transpose()?
            .flatten();
        match portal.as_deref() {
            Some(Portal::Encrypted { .. }) => {
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Encrypted");

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TandemConfig;
    use crate::postgresql::context::KeysetIdentifier;
    use crate::postgresql::parser::SqlParser;
    use crate::postgresql::rewrite::Name;
    use crate::postgresql::test_operation_id as operation_id;
    use crate::proxy::{EncryptConfig, EncryptionService};
    use cipherstash_client::schema::{ColumnConfig, ColumnType};
    use eql_mapper::Schema;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[derive(Clone)]
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

        async fn decrypt_inbound_eql(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(vec![])
        }
    }

    fn create_backend() -> Backend<TestService> {
        create_backend_with_context().0
    }

    fn create_backend_with_context() -> (Backend<TestService>, Context<TestService>) {
        create_backend_with_encrypt_config(EncryptConfig::default())
    }

    fn create_backend_with_encrypt_config(
        encrypt_config: EncryptConfig,
    ) -> (Backend<TestService>, Context<TestService>) {
        let config = Arc::new(TandemConfig::for_testing());
        let encrypt_config = Arc::new(encrypt_config);
        let schema = Arc::new(Schema::new("public"));
        let (reload_sender, _) = mpsc::unbounded_channel();
        let context = Context::new(
            1,
            config,
            encrypt_config,
            schema,
            Arc::new(rustls::RootCertStore::empty()),
            TestService {},
            reload_sender,
        );
        (Backend::new(context.clone()), context)
    }

    #[test]
    fn decryption_failure_is_returned_as_a_postgresql_error() {
        let mut backend = create_backend();

        let output = backend.decryption_failure(Error::Unknown);

        assert!(matches!(
            output,
            BackendMiddlewareOutput::Expand(messages)
                if matches!(messages.as_slice(), [BackendMessage::ErrorResponse(_)])
        ));
        assert!(backend.discard_execution);
    }

    #[tokio::test]
    async fn failed_transaction_readiness_is_forwarded_without_publishing_schema() {
        let config = Arc::new(TandemConfig::for_testing());
        let encrypt_config = Arc::new(EncryptConfig::default());
        let schema = Arc::new(Schema::new("public"));
        let (reload_sender, mut reload_receiver) = mpsc::unbounded_channel();
        let context = Context::new(
            1,
            config,
            encrypt_config,
            schema,
            Arc::new(rustls::RootCertStore::empty()),
            TestService {},
            reload_sender,
        );
        let ddl = SqlParser::parse_statement("create table reports (id bigint)").unwrap();
        context.execute_simple_schema_statements(&[ddl]);
        context.report_schema_execution_succeeded();

        let mut backend = Backend::new(context);
        let expected =
            BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::FailedTransaction);
        let output = backend.intercept(None, expected.clone()).await.unwrap();

        assert_eq!(output, BackendMiddlewareOutput::Forward(expected));
        assert!(reload_receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn database_error_without_an_execution_is_forwarded() {
        let mut backend = create_backend();
        let response = crate::postgresql::diagnostics::invalid_sql_statement(
            "syntax error at or near SELECT".to_owned(),
        );
        let message = BackendMessage::ErrorResponse(response.clone());

        let output = backend.intercept(None, message.clone()).await.unwrap();

        assert_eq!(output, BackendMiddlewareOutput::Forward(message));
    }

    #[tokio::test]
    async fn terminal_message_without_an_operation_is_forwarded() {
        let (mut backend, context) = create_backend_with_context();
        let ddl = SqlParser::parse_statement("create table reports (id bigint)").unwrap();
        context.execute_simple_schema_statements(&[ddl]);
        assert!(context.schema_ddl_in_flight());
        let message = BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SELECT 1"));

        let output = backend.intercept(None, message.clone()).await.unwrap();

        assert_eq!(output, BackendMiddlewareOutput::Forward(message));
        assert!(!context.schema_ddl_in_flight());
    }

    #[tokio::test]
    async fn correlated_terminal_message_for_an_unknown_operation_fails_closed() {
        let mut backend = create_backend();
        let message = BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SELECT 1"));

        let result = backend
            .intercept(Some(operation_id()), message.clone())
            .await;

        assert!(matches!(
            result,
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[tokio::test]
    async fn terminal_message_for_a_non_execution_operation_fails_closed() {
        let (mut backend, context) = create_backend_with_context();
        let operation = operation_id();
        context.set_non_execution(operation).unwrap();
        let message = BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SELECT 1"));

        let result = backend.intercept(Some(operation), message.clone()).await;

        assert!(matches!(
            result,
            Err(Error::Context(
                crate::error::ContextError::OperationWithoutExecute
            ))
        ));
    }

    #[tokio::test]
    async fn no_data_for_an_untracked_describe_fails_closed() {
        let mut backend = create_backend();

        let result = backend
            .intercept(Some(operation_id()), BackendMessage::NoData)
            .await;

        assert!(matches!(
            result,
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[tokio::test]
    async fn row_description_for_an_untracked_mapped_operation_fails_closed() {
        let mut encrypt_config = EncryptConfig::default();
        encrypt_config.insert(
            crate::Identifier::new("records", "secret"),
            ColumnConfig::build("secret".to_owned()).casts_as(ColumnType::Text),
        );
        let (mut backend, _) = create_backend_with_encrypt_config(encrypt_config);
        let message = BackendMessage::RowDescription(pg_proto::RowDescription { fields: vec![] });

        let result = backend
            .intercept(Some(operation_id()), message.clone())
            .await;

        assert!(matches!(
            result,
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[tokio::test]
    async fn data_row_for_an_unknown_correlated_operation_fails_closed() {
        let mut encrypt_config = EncryptConfig::default();
        encrypt_config.insert(
            crate::Identifier::new("records", "secret"),
            ColumnConfig::build("secret".to_owned()).casts_as(ColumnType::Text),
        );
        let (mut backend, _) = create_backend_with_encrypt_config(encrypt_config);

        let message = BackendMessage::DataRow(DataRow {
            columns: vec![Some(bytes::Bytes::from_static(b"ciphertext"))],
        });
        let result = backend
            .intercept(Some(operation_id()), message.clone())
            .await;

        assert!(matches!(
            result,
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
        assert!(!backend.discard_execution);
    }

    #[tokio::test]
    async fn database_error_for_a_non_execution_operation_is_forwarded() {
        let (mut backend, context) = create_backend_with_context();
        let operation = operation_id();
        context.set_non_execution(operation).unwrap();
        let response = crate::postgresql::diagnostics::invalid_sql_statement(
            "syntax error at or near SELECT".to_owned(),
        );
        let message = BackendMessage::ErrorResponse(response);

        let output = backend
            .intercept(Some(operation), message.clone())
            .await
            .unwrap();

        assert_eq!(output, BackendMiddlewareOutput::Forward(message));
    }

    #[tokio::test]
    async fn database_error_for_an_unregistered_operation_fails_closed() {
        let (mut backend, context) = create_backend_with_context();
        let ddl = SqlParser::parse_statement("create table reports (id bigint)").unwrap();
        context.execute_simple_schema_statements(&[ddl]);
        assert!(context.schema_ddl_in_flight());
        let message = BackendMessage::ErrorResponse(
            crate::postgresql::diagnostics::invalid_sql_statement("database error".to_owned()),
        );

        let result = backend
            .intercept(Some(operation_id()), message.clone())
            .await;

        assert!(matches!(
            result,
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
        assert!(!context.schema_ddl_in_flight());
    }

    #[tokio::test]
    async fn parse_completion_releases_non_execution_error_correlation() {
        let (mut backend, context) = create_backend_with_context();
        let operation = operation_id();
        context.set_non_execution(operation).unwrap();

        let output = backend
            .intercept(Some(operation), BackendMessage::ParseComplete)
            .await
            .unwrap();

        assert_eq!(
            output,
            BackendMiddlewareOutput::Forward(BackendMessage::ParseComplete)
        );
        assert!(matches!(
            context.finish_execution(operation, ExecutionOutcome::Failed),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[tokio::test]
    async fn correlated_non_execution_completion_for_an_unknown_operation_fails_closed() {
        for message in [BackendMessage::ParseComplete, BackendMessage::BindComplete] {
            let mut backend = create_backend();

            let result = backend.intercept(Some(operation_id()), message).await;

            assert!(matches!(
                result,
                Err(Error::Context(crate::error::ContextError::UnknownOperation))
            ));
        }
    }

    #[tokio::test]
    async fn stored_proxy_error_replaces_a_non_execution_database_error() {
        let (mut backend, mut context) = create_backend_with_context();
        let operation = operation_id();
        let replacement =
            crate::postgresql::diagnostics::invalid_sql_statement("proxy parse error".to_owned());
        context
            .set_operation_error(operation, replacement.clone())
            .unwrap();
        let database_error =
            BackendMessage::ErrorResponse(crate::postgresql::diagnostics::invalid_sql_statement(
                "database parse error".to_owned(),
            ));

        let output = backend
            .intercept(Some(operation), database_error)
            .await
            .unwrap();

        assert_eq!(
            output,
            BackendMiddlewareOutput::Forward(BackendMessage::ErrorResponse(replacement))
        );
    }

    #[tokio::test]
    async fn successful_execution_is_not_replaced_by_a_stored_error() {
        let (mut backend, mut context) = create_backend_with_context();
        let operation = operation_id();
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_execute(operation, Name::new(), Some(scope))
            .unwrap();
        context
            .set_operation_error(
                operation,
                crate::postgresql::diagnostics::invalid_sql_statement("proxy error".to_owned()),
            )
            .unwrap();
        let message = BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SELECT 1"));

        let output = backend
            .intercept(Some(operation), message.clone())
            .await
            .unwrap();

        assert_eq!(output, BackendMiddlewareOutput::Forward(message));
    }

    #[tokio::test]
    async fn decryption_recovery_discards_suppressed_execution_state() {
        let mut encrypt_config = EncryptConfig::default();
        encrypt_config.insert(
            crate::Identifier::new("records", "secret"),
            ColumnConfig::build("secret".to_owned()).casts_as(ColumnType::Text),
        );
        let (mut backend, mut context) = create_backend_with_encrypt_config(encrypt_config);
        let operation = operation_id();
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_execute(operation, Name::new(), Some(scope))
            .unwrap();
        backend.discard_execution = true;
        let message = BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SELECT 1"));

        let output = backend
            .intercept(Some(operation), message.clone())
            .await
            .unwrap();

        assert_eq!(output, BackendMiddlewareOutput::Suppress(message));
        assert!(matches!(
            context.get_execute(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
        assert!(context.get_metrics_scope(scope).unwrap().is_none());
    }

    #[tokio::test]
    async fn discarded_execution_response_resolves_its_schema_execution() {
        let mut encrypt_config = EncryptConfig::default();
        encrypt_config.insert(
            crate::Identifier::new("records", "secret"),
            ColumnConfig::build("secret".to_owned()).casts_as(ColumnType::Text),
        );
        let (mut backend, mut context) = create_backend_with_encrypt_config(encrypt_config);
        let operation = operation_id();
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_execute(operation, Name::new(), Some(scope))
            .unwrap();
        let ddl = SqlParser::parse_statement("create table reports (id bigint)").unwrap();
        context.execute_simple_schema_statements(&[ddl]);
        assert!(context.schema_ddl_in_flight());
        backend.discard_execution = true;

        let message = BackendMessage::CommandComplete(bytes::Bytes::from_static(b"CREATE TABLE"));
        let output = backend
            .intercept(Some(operation), message.clone())
            .await
            .unwrap();

        assert_eq!(output, BackendMiddlewareOutput::Suppress(message));
        assert!(!context.schema_ddl_in_flight());
    }

    #[tokio::test]
    async fn uncorrelated_flush_failure_returns_a_postgresql_error() {
        let mut encrypt_config = EncryptConfig::default();
        encrypt_config.insert(
            crate::Identifier::new("records", "secret"),
            ColumnConfig::build("secret".to_owned()).casts_as(ColumnType::Text),
        );
        let (mut backend, mut context) = create_backend_with_encrypt_config(encrypt_config);
        let operation = operation_id();
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_execute(operation, Name::new(), Some(scope))
            .unwrap();
        let ddl = SqlParser::parse_statement("create table reports (id bigint)").unwrap();
        context.execute_simple_schema_statements(&[ddl]);
        assert!(context.schema_ddl_in_flight());
        backend.encrypted_rows_operation = Some(operation);
        backend.encrypted_rows.push(DataRow {
            columns: vec![Some(bytes::Bytes::from_static(b"invalid ciphertext"))],
        });

        let output = backend
            .intercept(
                None,
                BackendMessage::CommandComplete(bytes::Bytes::from_static(b"CREATE TABLE")),
            )
            .await
            .unwrap();

        assert!(matches!(
            output,
            BackendMiddlewareOutput::Expand(messages)
                if matches!(messages.as_slice(), [BackendMessage::ErrorResponse(_)])
        ));
        assert!(backend.discard_execution);
        assert!(matches!(
            context.get_execute(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
        assert!(context.get_metrics_scope(scope).unwrap().is_none());
        assert!(context.schema_ddl_in_flight());
    }

    #[tokio::test]
    async fn suspended_portal_resolves_one_schema_execution() {
        let (mut backend, context) = create_backend_with_context();
        let ddl = SqlParser::parse_statement("create table reports (id bigint)").unwrap();
        context.execute_simple_schema_statements(&[ddl]);
        assert!(context.schema_ddl_in_flight());

        let output = backend
            .intercept(None, BackendMessage::PortalSuspended)
            .await
            .unwrap();

        assert_eq!(
            output,
            BackendMiddlewareOutput::Forward(BackendMessage::PortalSuspended)
        );
        assert!(!context.schema_ddl_in_flight());
    }

    #[tokio::test]
    async fn publication_failure_closes_connection_before_idle_readiness() {
        let config = Arc::new(TandemConfig::for_testing());
        let encrypt_config = Arc::new(EncryptConfig::default());
        let schema = Arc::new(Schema::new("public"));
        let (reload_sender, mut reload_receiver) = mpsc::unbounded_channel();
        let context = Context::new(
            1,
            config,
            encrypt_config,
            schema,
            Arc::new(rustls::RootCertStore::empty()),
            TestService {},
            reload_sender,
        );
        let ddl = SqlParser::parse_statement("create table reports (id bigint)").unwrap();
        context.execute_simple_schema_statements(&[ddl]);
        context.report_schema_execution_succeeded();

        let reload_task = tokio::spawn(async move {
            let Some(crate::proxy::ReloadCommand::DatabaseSchema(responder)) =
                reload_receiver.recv().await
            else {
                panic!("expected a database schema reload command");
            };
            responder.send(false).unwrap();
        });

        let mut backend = Backend::new(context);
        let ready = BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle);
        let result = backend.intercept(None, ready).await;
        reload_task.await.unwrap();
        assert!(result.is_err());
    }
}
