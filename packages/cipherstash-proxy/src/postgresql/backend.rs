use super::context::Context;
use super::data::to_sql;
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
use tracing::{debug, error, info};

pub struct Backend<R>
where
    R: AsyncRead + Unpin,
{
    client_sender: Sender,
    server_reader: R,
    encrypt: Encrypt,
    context: Context,
    buffer: MessageBuffer,
}

impl<R> Backend<R>
where
    R: AsyncRead + Unpin,
{
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

    ///
    /// An Execute phase is always terminated by the appearance of exactly one of these messages:
    ///     CommandComplete
    ///     EmptyQueryResponse
    ///     ErrorResponse
    ///     PortalSuspended
    ///
    /// Describe Flow
    ///     Describe => [D1, D2]
    ///         Get -> D1
    ///         Handle ParameterDescription and/or RowDescription
    ///         Complete
    ///     Describe => [D2]
    ///
    /// Execute Flow
    ///     Execute [E1, E2]
    ///        Get -> E1
    ///        Handle DataRow
    ///               DataRow
    ///
    ///
    ///
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
                self.flush().await?;
                self.context.complete_execution();
            }
            BackendCode::ErrorResponse => {
                if let Some(b) = self.error_response_handler(&bytes)? {
                    bytes = b
                }
                self.flush().await?;
                self.context.complete_execution();
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

            _ => {}
        }

        self.write_with_flush(bytes).await?;

        Ok(())
    }

    ///
    /// Handle error response messages
    /// The Frontend triggers an exception in the database for some errors
    /// These errors are filtered here
    /// Other Error Responses are logged and passed through
    ///
    fn error_response_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        let error_response = ErrorResponse::try_from(bytes)?;
        error!(msg = "PostgreSQL Error", error = ?error_response);
        info!(msg = "PostgreSQL Errors are returned from the database");
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
        self.flush().await?;

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

    ///
    /// Flush all buffered DataRow messages
    ///
    /// Decrypts any configured column values and writes the decrypted values to the client
    ///
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

        // Decrypt CipherText -> Plaintext
        let plaintexts = self.encrypt.decrypt(ciphertexts).await.inspect_err(|_| {
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

    ///
    /// Handle DataRow messages
    /// If there is no associated Portal, the row does not require decryption and can be passed through
    ///
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
