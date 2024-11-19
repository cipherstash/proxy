use bytes::{BufMut, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tracing::debug;

use super::protocol::{self};
use crate::encrypt::Encrypt;
use crate::error::Error;
use crate::postgresql::{Bind, CONNECTION_TIMEOUT, PROTOCOL_VERSION_NUMBER};
use crate::SIZE_I32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Code {
    Query,
    Parse,
    Bind,
    Unknown(char),
}

pub struct Frontend<C, S>
where
    C: AsyncRead + Unpin,
    S: AsyncWriteExt + Unpin,
{
    client: C,
    server: S,
    encrypt: Encrypt,
    startup_complete: bool,
}

impl<C, S> Frontend<C, S>
where
    C: AsyncRead + Unpin,
    S: AsyncWriteExt + Unpin,
{
    pub fn new(client: C, server: S, encrypt: Encrypt) -> Self {
        Frontend {
            client,
            server,
            encrypt,
            startup_complete: false,
        }
    }

    pub async fn write(&mut self, bytes: BytesMut) -> Result<(), Error> {
        // debug!("[frontend.write]");
        self.server.write_all(&bytes).await?;
        Ok(())
    }

    pub async fn read(&mut self) -> Result<BytesMut, Error> {
        if !self.startup_complete {
            let bytes = self.read_start_up_message().await?;
            return Ok(bytes);
        }

        debug!("[frontend.read]");
        let mut message =
            timeout(CONNECTION_TIMEOUT, protocol::read_message(&mut self.client)).await??;

        match message.code.into() {
            Code::Query => {
                // debug!("Query");
                // let query = Query::try_from(&message.bytes.clone())?;
                // debug!("{query:?}");
            }
            Code::Parse => {
                // debug!("Parse");
                // let parse = Parse::try_from(&message.bytes)?;
                // debug!("{parse:?}");
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

    async fn bind_handler(
        &mut self,
        message: &protocol::Message,
    ) -> Result<Option<BytesMut>, Error> {
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

    ///
    /// Read the start up message from the client
    /// Startup messages are sent by the client to the server to initiate a connection
    ///
    async fn read_start_up_message(&mut self) -> Result<BytesMut, Error> {
        let len = self.client.read_i32().await?;
        debug!("[read_start_up_message]");

        let capacity = len as usize;

        let mut bytes = BytesMut::with_capacity(capacity);
        bytes.put_i32(len);
        bytes.resize(capacity, b'0');

        let slice_start = SIZE_I32;
        self.client.read_exact(&mut bytes[slice_start..]).await?;

        // code is the first 4 bytes after len
        let code_bytes: [u8; 4] = [
            bytes.as_ref()[4],
            bytes.as_ref()[5],
            bytes.as_ref()[6],
            bytes.as_ref()[7],
        ];

        let code = i32::from_be_bytes(code_bytes);
        if code == PROTOCOL_VERSION_NUMBER {
            self.startup_complete = true;
        }

        Ok(bytes)
    }
}

impl From<u8> for Code {
    fn from(code: u8) -> Self {
        match code as char {
            'Q' => Code::Query,
            'P' => Code::Parse,
            'B' => Code::Bind,
            _ => Code::Unknown(code as char),
        }
    }
}
