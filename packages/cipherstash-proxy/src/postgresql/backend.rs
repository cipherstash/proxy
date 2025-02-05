use super::context::Context;
use super::data::to_sql;
use super::message_buffer::MessageBuffer;
use super::messages::error_response::ErrorResponse;
use super::messages::row_description::RowDescription;
use super::messages::BackendCode;
use crate::encrypt::Encrypt;
use crate::eql::Encrypted;
use crate::error::Error;
use crate::log::{DEVELOPMENT, MAPPER};
use crate::postgresql::context::Portal;
use crate::postgresql::messages::data_row::DataRow;
use crate::postgresql::messages::param_description::ParamDescription;
use crate::postgresql::protocol::{self};
use bytes::BytesMut;
use itertools::Itertools;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, warn};

pub struct Backend<C, S>
where
    C: AsyncWrite + Unpin,
    S: AsyncRead + Unpin,
{
    client: C,
    server: S,
    encrypt: Encrypt,
    context: Context,
    buffer: MessageBuffer,
}

impl<C, S> Backend<C, S>
where
    C: AsyncWrite + Unpin,
    S: AsyncRead + Unpin,
{
    pub fn new(client: C, server: S, encrypt: Encrypt, context: Context) -> Self {
        let buffer = MessageBuffer::new();
        Backend {
            client,
            server,
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

        let (code, mut bytes) = protocol::read_message_with_timeout(
            &mut self.server,
            self.context.client_id,
            connection_timeout,
        )
        .await?;

        if self.encrypt.is_passthrough() {
            self.write(bytes).await?;
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
                debug!(target: DEVELOPMENT, client_id = self.context.client_id, msg = "CommandComplete | EmptyQueryResponse | PortalSuspended");
                self.flush().await?;
                self.context.complete_execution();
            }
            BackendCode::ErrorResponse => {
                self.error_response_handler(&bytes)?;
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
            _ => {}
        }

        self.write(bytes).await?;

        Ok(())
    }

    ///
    /// Handle error response messages
    /// Error Responses are not rewritten, we log the error for ease of use
    ///
    fn error_response_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let error_response = ErrorResponse::try_from(bytes)?;
        error!(msg = "PostgreSQL Error", error = ?error_response);
        warn!("Error response originates in the PostgreSQL database.");
        Ok(())
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
    ///
    /// Flushes any nessages in the buffer before writing the message
    ///
    pub async fn write(&mut self, bytes: BytesMut) -> Result<(), Error> {
        debug!(target: DEVELOPMENT, client_id = self.context.client_id, msg = "Write");
        self.flush().await?;
        self.client.write_all(&bytes).await?;
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
            Some(Portal::Encrypted(portal)) => portal,
            _ => {
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Passthrough");
                return Ok(());
            }
        };

        let rows: Vec<DataRow> = self.buffer.drain().into_iter().collect();
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

        // Each row is converted into Vec<Option<CipherText>>
        let ciphertexts: Vec<Option<Encrypted>> = rows
            .iter()
            .map(|row| row.to_ciphertext())
            .flatten_ok()
            .collect::<Result<Vec<_>, _>>()?;

        // Decrypt CipherText -> Plaintext
        let plaintexts = self.encrypt.decrypt(ciphertexts).await?;

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
            self.client.write_all(&bytes).await?;
        }

        Ok(())
    }

    async fn parameter_description_handler(
        &self,
        bytes: &BytesMut,
    ) -> Result<Option<BytesMut>, Error> {
        let mut description = ParamDescription::try_from(bytes)?;

        if let Some(statement) = self.context.get_statement_from_describe() {
            let param_types = statement
                .param_columns
                .iter()
                .map(|col| col.as_ref().map(|col| col.postgres_type.clone()))
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
        match self.context.get_portal_from_execute().as_deref() {
            Some(Portal::Encrypted(portal)) => {
                debug!(target: MAPPER, client_id = self.context.client_id, portal = ?portal);
                let data_row = DataRow::try_from(bytes)?;
                self.buffer(data_row).await?;
                Ok(true)
            }
            _ => {
                debug!(target: MAPPER, client_id = self.context.client_id, msg = "Passthrough");
                Ok(false)
            }
        }
    }
}
