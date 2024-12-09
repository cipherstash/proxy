use crate::encrypt::Encrypt;
use crate::error::Error;
use crate::postgresql::protocol::{self};
use crate::postgresql::CONNECTION_TIMEOUT;
use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tracing::{error, warn};

use super::messages::error_response::ErrorResponse;
use super::messages::BackendCode;
use super::protocol::Message;

pub struct Backend<C, S>
where
    C: AsyncWrite + Unpin,
    S: AsyncRead + Unpin,
{
    client: C,
    server: S,
    encrypt: Encrypt,
}

impl<C, S> Backend<C, S>
where
    C: AsyncWrite + Unpin,
    S: AsyncRead + Unpin,
{
    pub fn new(client: C, server: S, encrypt: Encrypt) -> Self {
        Backend {
            client,
            server,
            encrypt,
        }
    }

    pub async fn rewrite(&mut self) -> Result<(), Error> {
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
        let message =
            timeout(CONNECTION_TIMEOUT, protocol::read_message(&mut self.server)).await??;

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
