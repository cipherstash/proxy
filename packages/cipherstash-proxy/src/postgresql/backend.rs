use super::context::Context;
use super::data::to_sql;
use super::message_buffer::MessageBuffer;
use super::messages::error_response::ErrorResponse;
use super::messages::row_description::RowDescription;
use super::messages::BackendCode;
use crate::connect::Sender;
use crate::encrypt::Encrypt;
use crate::eql::Encrypted;
use crate::error::Error;
use crate::log::{CONFIG, DEVELOPMENT, MAPPER, PROTOCOL};
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
use itertools::Itertools;
use metrics::{counter, histogram};
use std::time::Instant;
use tokio::io::AsyncRead;
use tracing::{debug, error, info, warn};

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

        self.warn_if_encrypted_value_with_no_config(&code, &bytes)?;

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
            Some(Portal::Encrypted { .. }) | Some(Portal::EncryptedText) => portal.unwrap(),
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

        let start = Instant::now();

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
        counter!(ROWS_TOTAL).increment(1);
        match self.context.get_portal_from_execute().as_deref() {
            Some(Portal::Encrypted { .. }) | Some(Portal::EncryptedText) => {
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

    // Wrapper method for the struct-level function
    fn warn_if_encrypted_value_with_no_config(
        &self,
        code: &u8,
        bytes: &BytesMut,
    ) -> Result<(), Error> {
        Self::_warn_if_encrypted_value_with_no_config(
            code,
            &self.context.client_id,
            bytes,
            self.encrypt.encrypt_config.is_empty(),
            self.encrypt.config.mapping_disabled(),
        )
    }

    // Actual logic for warning when a column looks like encrypted with no encrypt config.
    // This is a struct function so it can be called without setting up a full backend.
    fn _warn_if_encrypted_value_with_no_config(
        code: &u8,
        client_id: &i32,
        bytes: &BytesMut,
        config_empty: bool,
        mapping_disabled: bool,
    ) -> Result<(), Error> {
        // if explicitly disabled, no warning
        // if config is not missing, no warning
        if mapping_disabled || !config_empty {
            return Ok(());
        }

        if let BackendCode::DataRow = (*code).into() {
            let data_row = DataRow::try_from(bytes)?;
            let has_encrypted_field = data_row
                .columns
                .iter()
                .any(|c| Option::<crate::eql::Encrypted>::from(c).is_some());

            if has_encrypted_field {
                warn!(target: CONFIG,
                    client_id = client_id,
                    msg = "WARNING: Possible encrypted column with no encrypt config found",
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use tracing::dispatcher::set_default;
    use tracing::subscriber::DefaultGuard;

    use crate::config::{LogConfig, LogLevel};
    use crate::connect::AsyncStream;
    use crate::log::log_test_helper::MockMakeWriter;
    use crate::log::set_format;
    use crate::log::subscriber;
    use crate::postgresql::backend::Backend;
    use crate::postgresql::messages::BackendCode;
    use tracing_subscriber::fmt::writer::BoxMakeWriter;

    fn test_bytes() -> BytesMut {
        let bytes_source: &[u8] = b"D\0\0\x04V\0\x01\0\0\x04L{\"c\": \"mBbM1A=57k7WyKj<{Oo&6@OyH6Sh<!B`<ojz7ZzjCB?I>rIm=D#2_crSAUhqS4lSrCHR79sChoeo8k6Nu_>VX$0_*@q-U;WZewzJaCBv4Ut(`>Y`_\", \"i\": {\"c\": \"encrypted_bool\", \"t\": \"encrypted\"}, \"k\": \"ct\", \"m\": null, \"o\": [\"ccccccccccccccccd23d6a4c3eee512713175e673c6d995ff5d9b1d3492fe8eb289c3eb95029025f5b71fc6e06632b4a1302980e433361c7999724dbdd052739258d9444b0fbd43cc41f1f3e8ace7a5639afaea11d4dc3e5e4d846418a11c5a7fb7d32dbdbe843787ed8b61267cb3e59b064dc5f935da02b578fcc54c40a053de6c833086a62f23e49e7a041882ceea1a1e65d327911526f0667c1f6f343839f16fd5731089ca29d278398461c8a95fe1158e7b78d64ccd5d181c2b0c65ccbd7b71ca28251a0393a65fb79a90ed19063e2b5e30155c7940c70ac570a17d516fbc19dd2d6a27ab15f5cd6e2bc11c356a83e36f5f19528d192874d6a13e94492d9f4732056057be16ce9d5480073a71eed887195646eb9f76bdf769a3188fdd3528c39738390dd5f02c4476b317be1a55fa77d531e39469ca26e800b0f766d84c2f8a78ca8c3e7afc52e7bfd52d67be2bdc7655f8207af9a2f72141dde6e2148b1eaad7235caee1fd3bb3ac62993a62c411e31c87ae65a00defff305a99223740356d5e596fb01f83fe30ad8e2d8d67aaee128615bf2a9f43b\"], \"u\": \"cceee631ea969008d5ffd4233002dc76da162d99a66107eed2d4fdc9c9dbd2d5\", \"v\": 1}";
        BytesMut::from(bytes_source)
    }

    fn set_up_logging(make_writer: &MockMakeWriter) -> DefaultGuard {
        let config = LogConfig::with_level(LogLevel::Warn);
        let subscriber =
            subscriber::builder(&config).with_writer(BoxMakeWriter::new(make_writer.clone()));
        let subscriber = set_format(&config, subscriber);
        set_default(&subscriber.into())
    }

    // warning is emitted if encrypt config is missing (implicitly passthrough) and mapping is not explicitly disabled
    #[test]
    fn warn_if_encrypt_config_missing_and_mapping_not_disabled() {
        let make_writer = MockMakeWriter::default();
        let _logging = set_up_logging(&make_writer);
        let bytes = test_bytes();

        let results = Backend::<AsyncStream>::_warn_if_encrypted_value_with_no_config(
            &BackendCode::DataRow.into(),
            &123,
            &bytes,
            true,
            false,
        );

        assert!(results.is_ok());

        let log_contents = make_writer.get_string();
        assert!(log_contents
            .contains("WARNING: Possible encrypted column with no encrypt config found"));
    }

    // warning is not emitted if encryption is explicitly disabled
    #[test]
    fn no_warn_if_encryption_is_explicitly_disabled() {
        let make_writer = MockMakeWriter::default();
        let _logging = set_up_logging(&make_writer);
        let bytes = test_bytes();

        let results = Backend::<AsyncStream>::_warn_if_encrypted_value_with_no_config(
            &BackendCode::DataRow.into(),
            &123,
            &bytes,
            false,
            true,
        );

        assert!(results.is_ok());

        let log_contents = make_writer.get_string();
        assert!(log_contents.is_empty());
    }

    // warning is not emitted if encrypt config is present and mapping is not disabled
    #[test]
    fn no_warn_if_encrypt_config_present_and_mapping_not_disabled() {
        let make_writer = MockMakeWriter::default();
        let _logging = set_up_logging(&make_writer);
        let bytes = test_bytes();

        let results = Backend::<AsyncStream>::_warn_if_encrypted_value_with_no_config(
            &BackendCode::DataRow.into(),
            &123,
            &bytes,
            false,
            false,
        );

        assert!(results.is_ok());

        let log_contents = make_writer.get_string();
        assert!(log_contents.is_empty());
    }

    // warning is not emitted if it does not look like an encrypted column (regular JSONB)
    #[test]
    fn no_warn_if_value_does_not_look_like_an_encrypted_column() {
        let make_writer = MockMakeWriter::default();
        let _logging = set_up_logging(&make_writer);
        let bytes_source: &[u8] =
            b"D\0\0\x04V\0\x01\0\0\0\x1E{\"id\": \"123\", \"name\": \"Alice\"}";
        let bytes = BytesMut::from(bytes_source);

        let results = Backend::<AsyncStream>::_warn_if_encrypted_value_with_no_config(
            &BackendCode::DataRow.into(),
            &123,
            &bytes,
            true,
            false,
        );

        assert!(results.is_ok());

        let log_contents = make_writer.get_string();
        assert!(log_contents.is_empty());
    }
}
