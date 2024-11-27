use std::io::Cursor;

use super::bind::Bind;
use super::messages::FrontendCode as Code;
use super::protocol::{self, Message};
use crate::encrypt::Encrypt;
use crate::error::Error;
use crate::postgresql::messages::parse::Parse;
use crate::postgresql::CONNECTION_TIMEOUT;
use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tracing::{debug, error, info};

pub struct Frontend<C, S>
where
    C: AsyncRead + Unpin,
    S: AsyncWrite + Unpin,
{
    client: C,
    server: S,
    encrypt: Encrypt,
}

impl<C, S> Frontend<C, S>
where
    C: AsyncRead + Unpin,
    S: AsyncWrite + Unpin,
{
    pub fn new(client: C, server: S, encrypt: Encrypt) -> Self {
        Frontend {
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

    pub async fn write(&mut self, bytes: BytesMut) -> Result<(), Error> {
        self.server.write_all(&bytes).await?;
        Ok(())
    }

    pub async fn read(&mut self) -> Result<BytesMut, Error> {
        // debug!("[frontend] read");

        let mut message =
            timeout(CONNECTION_TIMEOUT, protocol::read_message(&mut self.client)).await??;

        match message.code.into() {
            Code::Query => {}
            Code::Parse => {
                if let Some(bytes) = self.parse_handler(&message).await? {
                    message.bytes = bytes;
                }
            }
            Code::Bind => {
                if let Some(bytes) = self.bind_handler(&message).await? {
                    message.bytes = bytes;
                }
            }
            code => {
                // debug!("Code {code:?}");
            }
        }

        Ok(message.bytes)
    }

    async fn parse_handler(&mut self, message: &Message) -> Result<Option<BytesMut>, Error> {
        debug!("Parse =====================");

        let mut parse = Parse::try_from(&message.bytes)?;

        debug!("AST =====================");

        debug!("AST =====================");

        let bytes = BytesMut::try_from(parse)?;

        Ok(Some(bytes))
    }

    async fn bind_handler(&mut self, message: &Message) -> Result<Option<BytesMut>, Error> {
        let mut bind = Bind::try_from(&message.bytes)?;

        let params = bind.to_plaintext()?;
        let encrypted = self.encrypt.encrypt(params).await?;

        bind.from_ciphertext(encrypted)?;

        if bind.should_rewrite() {
            let bytes = BytesMut::try_from(bind)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }
}
