use super::context::Context;
use super::data::to_sql;
use super::error_handler::PostgreSqlErrorHandler;
use super::message_buffer::MessageBuffer;
use super::messages::error_response::ErrorResponse;
use super::messages::row_description::RowDescription;
use super::messages::BackendCode;
use super::Column;
use crate::connect::Sender;
use crate::encrypt::Encrypt;
use crate::eql::EqlEncrypted;
use crate::error::{EncryptError, Error};
use crate::log::{DEVELOPMENT, MAPPER, PROTOCOL};
use crate::postgresql::context::Portal;
use crate::postgresql::messages::data_row::DataRow;
use crate::postgresql::messages::param_description::ParamDescription;
use crate::postgresql::protocol::{self};
use crate::prometheus::{
    CLIENTS_BYTES_SENT_TOTAL, DECRYPTED_VALUES_TOTAL, DECRYPTION_DURATION_SECONDS,
    DECRYPTION_ERROR_TOTAL, DECRYPTION_REQUESTS_TOTAL, ROWS_ENCRYPTED_TOTAL,
    ROWS_PASSTHROUGH_TOTAL, ROWS_TOTAL, SERVER_BYTES_RECEIVED_TOTAL,
};
use bytes::BytesMut;
use metrics::{counter, histogram};
use std::time::Instant;
use tokio::io::AsyncRead;
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
pub struct Backend<R>
where
    R: AsyncRead + Unpin,
{
    /// Sender for outgoing messages to client
    client_sender: Sender,
    /// Reader for incoming messages from server
    server_reader: R,
    /// Encryption service for column decryption
    encrypt: Encrypt,
    /// Session context with portal and statement metadata
    context: Context,
    /// Buffer for batching DataRow messages before decryption
    buffer: MessageBuffer,
}

impl<R> Backend<R>
where
    R: AsyncRead + Unpin,
{
    /// Creates a new Backend instance.
    ///
    /// # Arguments
    ///
    /// * `client_sender` - Channel sender for sending messages to the client
    /// * `server_reader` - Stream for reading messages from the PostgreSQL server
    /// * `encrypt` - Encryption service for handling column decryption
    /// * `context` - Session context shared with the frontend
    pub fn new(
        client_sender: Sender,
        server_reader: R,
        encrypt: Encrypt,
        context: Context,
    ) -> Self {
        let buffer = MessageBuffer::new();
        Backend {
            client_sender,
            server_reader,
            encrypt,
            context,
            buffer,
        }
    }

    /// Main message processing loop for handling server messages.
    ///
    /// Reads messages from the PostgreSQL server, processes them based on message type,
    /// performs decryption for encrypted result data, and forwards messages to the client.
    ///
    /// # PostgreSQL Protocol Phases
    ///
    /// ## Execute Phase
    /// Execute operations produce a stream of DataRow messages followed by exactly one of:
    /// - `CommandComplete` - Successful completion
    /// - `EmptyQueryResponse` - Empty query completed
    /// - `ErrorResponse` - Error occurred
    /// - `PortalSuspended` - Portal execution suspended (LIMIT reached)
    ///
    /// ## Describe Phase
    /// Describe operations return metadata about statements or portals:
    /// - `ParameterDescription` - Parameter metadata (for statements)
    /// - `RowDescription` - Result column metadata
    /// - `NoData` - No result columns
    ///
    /// # Message Processing Flow
    ///
    /// 1. **Read Message**: Read and parse PostgreSQL wire protocol message
    /// 2. **Check Passthrough**: Skip processing if encryption is disabled
    /// 3. **Handle by Type**: Route to appropriate handler based on message code
    /// 4. **Buffer Management**: Buffer DataRows, flush on completion/errors
    /// 5. **Forward**: Send processed message to PostgreSQL client
    ///
    /// # Buffering Behavior
    ///
    /// DataRow messages are buffered for batch decryption to improve performance.
    /// The buffer is automatically flushed when:
    /// - Buffer reaches capacity
    /// - Execute phase completes (CommandComplete, ErrorResponse, etc.)
    /// - Non-DataRow message is encountered
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful message processing, or an `Error` if a fatal
    /// error occurs that should terminate the connection.
    pub async fn rewrite(&mut self) -> Result<(), Error> {
        let connection_timeout = self.encrypt.config.database.connection_timeout();

        let (code, mut bytes) = protocol::read_message(
            &mut self.server_reader,
            self.context.client_id,
            connection_timeout,
        )
        .await?;

        let sent: u64 = bytes.len() as u64;
        counter!(SERVER_BYTES_RECEIVED_TOTAL).increment(sent);

        if self.encrypt.is_passthrough() {
            debug!(target: DEVELOPMENT,
                client_id = self.context.client_id,
                msg = "Passthrough enabled"
            );
            self.write_with_flush(bytes).await?;
            return Ok(());
        }

        let keyset_id = self.context.keyset_id();
        warn!(?self.context.client_id, ?keyset_id);

        match code.into() {
            BackendCode::DataRow => {
                // Encrypted DataRows are added to the buffer and we return early
                // Otherwise, continue and write
                if self.data_row_handler(&bytes).await? {
                    return Ok(());
                }
            }

            // Execute phase is always terminated by the appearance of exactly one of these messages:
            //      CommandComplete, EmptyQueryResponse (if the portal was created from an empty query string), ErrorResponse, or PortalSuspended.
            BackendCode::CommandComplete
            | BackendCode::EmptyQueryResponse
            | BackendCode::PortalSuspended => {
                debug!(target: PROTOCOL, client_id = self.context.client_id, msg = "CommandComplete | EmptyQueryResponse | PortalSuspended");

                match self.flush().await {
                    Ok(_) => (),
                    Err(err) => {
                        warn!(client_id = self.client_id(), error = err.to_string());
                        self.send_error_response(err)?;
                    }
                }

                self.context.complete_execution();
                self.context.finish_session();
            }
            BackendCode::ErrorResponse => {
                if let Some(b) = self.error_response_handler(&bytes)? {
                    bytes = b
                }

                match self.flush().await {
                    Ok(_) => (),
                    Err(err) => {
                        warn!(client_id = self.client_id(), error = err.to_string());
                        self.send_error_response(err)?;
                    }
                }

                self.context.complete_execution();
                self.context.finish_session();
            }
            // Describe with Target:Statement
            // Returns a ParameterDescription followed by RowDescription
            // The Describe is complete after the RowDescription
            BackendCode::ParameterDescription => {
                if let Some(b) = self.parameter_description_handler(&bytes).await? {
                    bytes = b
                }
            }
            // Describe with Target:Statement or Target::Portal
            // Target:Statement returns a ParameterDescription before a RowDescription
            // Target::Portal returns a RowDescription
            // If no rows are returned, NoData is returned instead of a RowDescription
            // Complete the Describe
            BackendCode::RowDescription => {
                if let Some(b) = self.row_description_handler(&bytes).await? {
                    bytes = b
                }
                self.context.complete_describe();
            }
            // Describe with Target:Statement or Target::Portal
            // If the statement returns no rows, NoData is returned instead of a RowDescription
            BackendCode::NoData => {
                self.context.complete_describe();
            }
            // Reload for SompleQuery flow
            // Reload is potentially triggered by a FrontEnd Sync message.
            // However, the SimpleQuery flow does not use Sync so we check here as well
            BackendCode::ReadyForQuery => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    msg = "ReadyForQuery"
                );
                if self.context.schema_changed() {
                    self.encrypt.reload_schema().await;
                }
            }

            code => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    msg = "Passthrough",
                    ?code,
                );
            }
        }

        self.write_with_flush(bytes).await?;

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
    fn error_response_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        let error_response = ErrorResponse::try_from(bytes)?;
        error!(msg = "PostgreSQL Error", error = ?error_response);
        info!(msg = "PostgreSQL Errors originate in the database");
        Ok(Some(bytes.to_owned()))
    }

    ///
    /// DataRows are buffered so that Decryption can be batched
    /// Decryption will occur
    ///  - on direct call to flush()
    ///  - when the buffer is full
    ///  - when any other message type is written
    ///
    async fn buffer(&mut self, data_row: DataRow) -> Result<(), Error> {
        self.buffer.push(data_row);
        if self.buffer.at_capacity() {
            debug!(target: DEVELOPMENT, client_id = self.context.client_id, msg = "Flush message buffer");
            self.flush().await?;
        }
        Ok(())
    }

    ///
    /// Write a message to the client
    /// Flushes all messages in the buffer before writing the message
    ///
    pub async fn write_with_flush(&mut self, bytes: BytesMut) -> Result<(), Error> {
        debug!(target: DEVELOPMENT, client_id = self.context.client_id, msg = "Write");

        match self.flush().await {
            Ok(_) => (),
            Err(err) => {
                warn!(client_id = self.client_id(), error = err.to_string());
                self.send_error_response(err)?;
            }
        }

        self.write(bytes).await?;
        Ok(())
    }

    ///
    /// Write a message to the client
    ///
    pub async fn write(&mut self, bytes: BytesMut) -> Result<(), Error> {
        let sent: u64 = bytes.len() as u64;
        counter!(CLIENTS_BYTES_SENT_TOTAL).increment(sent);

        self.client_sender.send(bytes)?;
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
    async fn flush(&mut self) -> Result<(), Error> {
        if self.buffer.is_empty() {
            debug!(target: MAPPER, client_id = self.context.client_id, msg = "Empty buffer");
        }

        let portal = self.context.get_portal_from_execute();
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

        let mut rows: Vec<DataRow> = self.buffer.drain().into_iter().collect();
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
        let ciphertexts: Vec<Option<EqlEncrypted>> = rows
            .iter_mut()
            .flat_map(|row| row.as_ciphertext(projection_columns))
            .collect::<Vec<_>>();

        let start = Instant::now();

        self.check_column_config(projection_columns, &ciphertexts)?;

        let keyset_id = self.context.keyset_id();
        warn!(?keyset_id);

        // Decrypt CipherText -> Plaintext
        let plaintexts = self
            .encrypt
            .decrypt(keyset_id, ciphertexts)
            .await
            .inspect_err(|_| {
                counter!(DECRYPTION_ERROR_TOTAL).increment(1);
            })?;

        // Avoid the iter calculation if we can
        if self.encrypt.config.prometheus_enabled() {
            let decrypted_count =
                plaintexts
                    .iter()
                    .fold(0, |acc, o| if o.is_some() { acc + 1 } else { acc });

            counter!(DECRYPTION_REQUESTS_TOTAL).increment(1);
            counter!(DECRYPTED_VALUES_TOTAL).increment(decrypted_count);

            let duration = Instant::now().duration_since(start);
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

            let bytes = BytesMut::try_from(row)?;
            self.write(bytes).await?;
        }

        Ok(())
    }

    fn check_column_config(
        &mut self,
        projection_columns: &[Option<Column>],
        ciphertexts: &[Option<EqlEncrypted>],
    ) -> Result<(), Error> {
        for (col, ct) in projection_columns.iter().zip(ciphertexts) {
            match (col, ct) {
                (Some(col), Some(ct)) => {
                    if col.identifier != ct.identifier {
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
                        table: ct.identifier.table.to_owned(),
                        column: ct.identifier.column.to_owned(),
                    }
                    .into());
                }
            }
        }
        Ok(())
    }

    async fn parameter_description_handler(
        &self,
        bytes: &BytesMut,
    ) -> Result<Option<BytesMut>, Error> {
        let mut description = ParamDescription::try_from(bytes)?;

        debug!(target: PROTOCOL, client_id = self.context.client_id, ParamDescription = ?description);

        if let Some(statement) = self.context.get_statement_from_describe() {
            let param_types = statement
                .param_columns
                .iter()
                .map(|col| {
                    col.as_ref().map(|col| {
                        debug!(target: MAPPER, client_id = self.context.client_id, ColumnConfig = ?col);
                        col.postgres_type.clone()
                    })
                })
                .collect::<Vec<_>>();

            debug!(target: MAPPER, client_id = self.context.client_id, param_types = ?param_types);

            description.map_types(&param_types);
        }

        if description.requires_rewrite() {
            let bytes = BytesMut::try_from(description)?;
            debug!(target: MAPPER, client_id = self.context.client_id, msg = "Rewrite ParamDescription", bytes = ?bytes);
            Ok(Some(bytes))
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
        bytes: &BytesMut,
    ) -> Result<Option<BytesMut>, Error> {
        let mut description = RowDescription::try_from(bytes)?;

        debug!(target: PROTOCOL, client_id = self.context.client_id, RowDescription = ?description);

        if let Some(statement) = self.context.get_statement_from_describe() {
            let projection_types = statement
                .projection_columns
                .iter()
                .map(|col| col.as_ref().map(|col| col.postgres_type.clone()))
                .collect::<Vec<_>>();

            debug!(target: MAPPER, client_id = self.context.client_id, projection_types = ?projection_types);

            description.map_types(&projection_types);
        }

        if description.requires_rewrite() {
            let bytes = BytesMut::try_from(description)?;
            debug!(target: MAPPER, client_id = self.context.client_id, msg = "Rewrite RowDescription", bytes = ?bytes);
            Ok(Some(bytes))
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
    async fn data_row_handler(&mut self, bytes: &BytesMut) -> Result<bool, Error> {
        counter!(ROWS_TOTAL).increment(1);
        match self.context.get_portal_from_execute().as_deref() {
            Some(Portal::Encrypted { .. }) => {
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Encrypted");

                let data_row = DataRow::try_from(bytes)?;
                self.buffer(data_row).await?;

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
impl<R> PostgreSqlErrorHandler for Backend<R>
where
    R: AsyncRead + Unpin,
{
    fn client_sender(&mut self) -> &mut Sender {
        &mut self.client_sender
    }

    fn client_id(&self) -> i32 {
        self.context.client_id
    }

    /// Backend-specific error response handling.
    ///
    /// Unlike the frontend, the backend doesn't need to set an error state
    /// since errors during result processing should immediately terminate
    /// the current query execution.
    fn send_error_response(&mut self, err: Error) -> Result<(), Error> {
        let error_response = self.error_to_response(err);
        // Ensure any buffered data is cleared before sending error
        self.buffer.clear();

        let message = BytesMut::try_from(error_response)?;

        debug!(
            target: "PROTOCOL",
            client_id = self.context.client_id,
            msg = "backend_send_error_response",
            ?message,
        );

        self.client_sender.send(message)?;

        Ok(())
    }
}
