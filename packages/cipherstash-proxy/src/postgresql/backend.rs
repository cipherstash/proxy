use super::context::Context;
use super::messages::error_response::ErrorResponse;
use super::messages::BackendCode;
use super::protocol::Message;
use crate::encrypt::Encrypt;
use crate::error::Error;
use crate::log::DEVELOPMENT;
use crate::postgresql::protocol::{self};
use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{error, warn};

pub struct Backend<C, S>
where
    C: AsyncWrite + Unpin,
    S: AsyncRead + Unpin,
{
    client: C,
    server: S,
    encrypt: Encrypt,
    context: Context,
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
        let message =
            protocol::read_message_with_timeout(&mut self.server, connection_timeout).await?;

        match message.code.into() {
            BackendCode::Authentication => {}

            BackendCode::DataRow => {
                // debug!("DataRow");
            }
            BackendCode::ErrorResponse => {
                let _ = self.error_response_handler(&message)?;
            }
            BackendCode::RowDescription => {
                // debug!("RowDescription");
            }
            _ => {
                // debug!("Backend {code:?}");
            }
        }

        Ok(message.bytes)
    }

    pub async fn write(&mut self, bytes: BytesMut) -> Result<(), Error> {
        self.client.write_all(&bytes).await?;
        Ok(())
    }

    ///
    /// Handle error response messages
    /// Error Responses are not rewritten, we log the error and return None
    ///
    ///
    fn error_response_handler(&mut self, message: &Message) -> Result<Option<BytesMut>, Error> {
        let error_response = ErrorResponse::try_from(&message.bytes)?;
        error!("{}", error_response);
        warn!("Error response originates in the PostgreSQL database.");
        Ok(None)
    }
}
