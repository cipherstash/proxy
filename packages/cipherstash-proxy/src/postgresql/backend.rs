use super::context::Context;
use super::message_buffer::MessageBuffer;
use super::messages::error_response::ErrorResponse;
use super::messages::row_description::RowDescription;
use super::messages::BackendCode;
use crate::encrypt::Encrypt;
use crate::eql::Ciphertext;
use crate::error::{EncryptError, Error};
use crate::log::{DEVELOPMENT, MAPPER};
use crate::postgresql::format_code::FormatCode;
use crate::postgresql::messages::data_row::DataRow;
use crate::postgresql::messages::param_description::ParamDescription;
use crate::postgresql::protocol::{self};
use bytes::BytesMut;
use cipherstash_client::encryption::Plaintext;
use itertools::Itertools;
use postgres_types::{ToSql, Type};
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
    /// TODO: fix the structure once implementation stabilizes
    ///
    pub async fn rewrite(&mut self) -> Result<(), Error> {
        if self.encrypt.config.disable_mapping() {
            warn!(DEVELOPMENT, "Mapping is not enabled");
            return Ok(());
        }

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
                if let Some(bytes) = self.parameter_description_handler(&bytes).await? {
                    debug!(target: MAPPER, "Rewrite ParamDescription");
                    self.write(bytes).await?;
                } else {
                    self.write(bytes).await?;
                }
            }
            BackendCode::RowDescription => {
                if let Some(bytes) = self.row_description_handler(&bytes).await? {
                    debug!(target: MAPPER, "Rewrite ParamDescription");
                    self.write(bytes).await?;
                } else {
                    self.write(bytes).await?;
                }
            }
            _ => {
                self.write(bytes).await?;
            }
        }

        self.flush().await?;
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

        // FormatCodes should not be None at this point
        // FormatCodes will be:
        //  - empty, in which case assume Text
        //  - single value, in which case use this for all columns
        //  - multiple values, in which case use the value for each column
        let format_codes = match self.context.get_result_format_codes_for_execute() {
            Some(format_codes) => {
                let format_code = match format_codes.first() {
                    Some(code) => *code,
                    None => FormatCode::Text,
                };

                if format_codes.len() != row_len {
                    vec![format_code; row_len]
                } else {
                    format_codes
                }
            }
            None => vec![FormatCode::Text; row_len],
        };

        error!(target: MAPPER, "Result format_codes {format_codes:?}");

        let ciphertexts: Vec<Option<Ciphertext>> = rows
            .iter()
            .map(|row| row.to_ciphertext())
            .flatten_ok()
            .collect::<Result<Vec<_>, _>>()?;

        let plaintexts = self.encrypt.decrypt(ciphertexts).await?;

        let rows = plaintexts.chunks(row_len).zip(rows);

        for (chunk, mut row) in rows {
            let data = chunk
                .iter()
                .zip(format_codes.iter())
                .map(|(plaintext, format_code)| {
                    debug!(target: MAPPER, "format_code: {format_code:?}");
                    match plaintext {
                        Some(plaintext) => plaintext_to_bytes(plaintext, format_code),
                        None => Ok(None),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;

            // debug!(target: MAPPER, "Data: {data:?}");

            row.rewrite(&data)?;

            let bytes = BytesMut::try_from(row)?;
            self.client.write_all(&bytes).await?;
        }

        // I think this goes here
        self.context.execute_complete();

        Ok(())
    }

    async fn parameter_description_handler(
        &self,
        bytes: &BytesMut,
    ) -> Result<Option<BytesMut>, Error> {
        let mut description = ParamDescription::try_from(bytes)?;

        // warn!("PARAMETER_DESCRIPTION ==============================");
        // debug!("{:?}", bytes);

        if let Some(param_columns) = self.context.get_param_columns_for_describe() {
            debug!("{:?}", param_columns);
            let param_types = param_columns
                .iter()
                .map(|col| col.as_ref().map(|col| col.postgres_type.clone()))
                .collect::<Vec<_>>();
            description.map_types(&param_types);
            debug!(target: MAPPER, "Mapped ParamDescription {description:?}");
        }

        // debug!("Mapped {:?}", description);
        // warn!("/PARAMETER_DESCRIPTION ==============================");
        if description.should_rewrite() {
            let bytes = BytesMut::try_from(description)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    async fn row_description_handler(
        &mut self,
        bytes: &BytesMut,
    ) -> Result<Option<BytesMut>, Error> {
        let mut description = RowDescription::try_from(bytes)?;

        // warn!("ROWDESCRIPTION ==============================");
        // // warn!("{:?}", self.context);
        // debug!("{:?}", self.context.describe);
        // debug!("RowDescription: {:?}", description);

        if let Some(projection_cols) = self.context.get_projection_columns_for_describe() {
            let projection_types = projection_cols
                .iter()
                .map(|col| col.as_ref().map(|col| col.postgres_type.clone()))
                .collect::<Vec<_>>();
            description.map_types(&projection_types);
            debug!(target: MAPPER, "Mapped RowDescription {description:?}");
        }

        self.context.describe_complete();

        // warn!("/ ROWDESCRIPTION ==============================");

        // description.fields.iter_mut().for_each(|field| {
        //     if field.name == "email" {
        //         field.rewrite_type_oid(postgres_types::Type::TEXT);
        //     }
        // });

        if description.should_rewrite() {
            let bytes = BytesMut::try_from(description)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }
}

fn plaintext_to_bytes(
    plaintext: &Plaintext,
    format_code: &FormatCode,
) -> Result<Option<BytesMut>, Error> {
    let bytes = match format_code {
        FormatCode::Text => plaintext_to_text(plaintext)?,
        FormatCode::Binary => plaintext_to_binary(plaintext)?,
    };

    Ok(Some(bytes))
}

fn plaintext_to_text(plaintext: &Plaintext) -> Result<BytesMut, Error> {
    let s = match &plaintext {
        Plaintext::Utf8Str(Some(x)) => x.to_string(),
        Plaintext::Int(Some(x)) => x.to_string(),
        Plaintext::BigInt(Some(x)) => x.to_string(),
        Plaintext::BigUInt(Some(x)) => x.to_string(),
        Plaintext::Boolean(Some(x)) => x.to_string(),
        Plaintext::Decimal(Some(x)) => x.to_string(),
        Plaintext::Float(Some(x)) => x.to_string(),
        Plaintext::NaiveDate(Some(x)) => x.to_string(),
        Plaintext::SmallInt(Some(x)) => x.to_string(),
        Plaintext::Timestamp(Some(x)) => x.to_string(),
        _ => todo!(),
    };

    Ok(BytesMut::from(s.as_bytes()))
}

fn plaintext_to_binary(plaintext: &Plaintext) -> Result<BytesMut, Error> {
    let mut bytes = BytesMut::new();

    let result = match &plaintext {
        Plaintext::BigInt(x) => x.to_sql_checked(&Type::INT8, &mut bytes),
        Plaintext::Boolean(x) => x.to_sql_checked(&Type::BOOL, &mut bytes),
        Plaintext::Float(x) => x.to_sql_checked(&Type::FLOAT8, &mut bytes),
        Plaintext::Int(x) => x.to_sql_checked(&Type::INT4, &mut bytes),
        Plaintext::NaiveDate(x) => x.to_sql_checked(&Type::DATE, &mut bytes),
        Plaintext::SmallInt(x) => x.to_sql_checked(&Type::INT2, &mut bytes),
        Plaintext::Timestamp(x) => x.to_sql_checked(&Type::TIMESTAMPTZ, &mut bytes),
        Plaintext::Utf8Str(x) => x.to_sql_checked(&Type::TEXT, &mut bytes),
        Plaintext::JsonB(x) => x.to_sql_checked(&Type::JSONB, &mut bytes),
        // TODO: Implement these
        Plaintext::Decimal(_x) => unimplemented!(),
        Plaintext::BigUInt(_x) => unimplemented!(),
    };

    match result {
        Ok(_) => Ok(bytes),
        Err(_e) => Err(EncryptError::PlaintextCouldNotBeEncoded.into()),
    }
}
