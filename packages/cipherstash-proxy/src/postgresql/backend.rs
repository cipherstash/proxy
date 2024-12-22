use super::context::Context;
use super::message_buffer::MessageBuffer;
use super::messages::error_response::ErrorResponse;
use super::messages::row_description::RowDescription;
use super::messages::BackendCode;
use super::protocol::Message;
use crate::encrypt::Encrypt;
use crate::eql::Ciphertext;
use crate::error::Error;
use crate::log::DEVELOPMENT;
use crate::postgresql::messages::data_row::DataRow;
use crate::postgresql::messages::param_description::ParamDescription;
use crate::postgresql::protocol::{self};
use bytes::BytesMut;
use itertools::Itertools;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, info, warn};

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

    pub async fn rewrite(&mut self) -> Result<(), Error> {
        if self.encrypt.config.disable_mapping() {
            warn!(DEVELOPMENT, "Mapping is not enabled");
            return Ok(());
        }
        let bytes = self.read().await?;
        self.write(bytes).await?;
        Ok(())
    }

    ///
    /// Startup sequence:
    ///     Client: SSL Request
    ///     Server: SSL Response (single byte S or N)
    ///
    pub async fn read(&mut self) -> Result<BytesMut, Error> {
        // info!("[backend] read");
        let connection_timeout = self.encrypt.config.database.connection_timeout();

        let (code, bytes) =
            protocol::read_message_with_timeout(&mut self.server, connection_timeout).await?;

        match code.into() {
            BackendCode::DataRow => {
                let data_row = DataRow::try_from(&bytes)?;
                self.buffer(data_row).await?;
            }
            BackendCode::ErrorResponse => {
                self.error_response_handler(&bytes)?;
                self.write(bytes).await?;
            }
            BackendCode::ParameterDescription => {
                info!("ParameterDescription");

                if let Some(bytes) = self.parameter_description_handler(&bytes).await? {
                    self.write(bytes).await?;
                } else {
                    self.write(bytes).await?;
                }
            }
            BackendCode::RowDescription => {
                if let Some(bytes) = self.row_description_handler(&bytes).await? {
                    self.write(bytes).await?;
                } else {
                    self.write(bytes).await?;
                }
            }
            _ => {
                self.write(bytes).await?;
            }
        }

        Ok(())
    }

    ///
    /// Handle error response messages
    /// Error Responses are not rewritten, we log the error for ease of use
    ///
    fn error_response_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let error_response = ErrorResponse::try_from(bytes)?;
        error!("{}", error_response);
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
        self.buffer.push(data_row).await;
        if self.buffer.at_capacity().await {
            debug!(target: DEVELOPMENT, "Flush message buffer");
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
        let rows: Vec<DataRow> = self.buffer.drain().await.into_iter().collect();

        let row_len = match rows.first() {
            Some(row) => row.column_count(),
            None => return Ok(()),
        };

        let ciphertexts: Vec<Option<Ciphertext>> = rows
            .iter()
            .map(|row| row.to_ciphertext())
            .flatten_ok()
            .collect::<Result<Vec<_>, _>>()?;

        let plaintexts = self.encrypt.decrypt(ciphertexts).await?;

        let rows = plaintexts.chunks(row_len).into_iter().zip(rows);
        for (chunk, mut row) in rows {
            row.update_from_ciphertext(chunk)?;

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

        warn!("PARAMETER_DESCRIPTION ==============================");
        // debug!("{:?}", description);
        // debug!("{:?}", self.context);

        let describe = self.context.describe.load();
        let describe = describe.as_ref();

        if let Some(describe) = describe {
            debug!("{:?}", describe);
            if let Some(param_types) = self.context.get_param_types(&describe.name) {
                debug!("{:?}", param_types);
                description.map_types(&param_types);
            }
        }

        debug!("Mapped {:?}", description);

        warn!("/PARAMETER_DESCRIPTION ==============================");
        Ok(None)
    }

    async fn row_description_handler(&self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        let mut row_description = RowDescription::try_from(bytes)?;

        warn!("ROWDESCRIPTION ==============================");
        // warn!("{:?}", self.context);
        debug!("{:?}", self.context.describe);
        debug!("RowDescription: {:?}", row_description);

        // if let Some(statement) = self.context.get(&bind.prepared_statement) {
        //     warn!("==============================");
        //     warn!("{:?}", statement);
        //     warn!("==============================");

        //     // bind.params.iter().zip()
        //     // let config = self.encrypt.column_config()
        // }

        warn!("/ ROWDESCRIPTION ==============================");

        row_description.fields.iter_mut().for_each(|field| {
            if field.name == "email" {
                field.rewrite_type_oid(postgres_types::Type::TEXT);
            }
        });

        if row_description.should_rewrite() {
            let bytes = BytesMut::try_from(row_description)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }
}
