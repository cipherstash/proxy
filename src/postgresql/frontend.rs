use super::bind::Bind;
use super::protocol::{self, Message};
use crate::encrypt::Encrypt;
use crate::error::Error;
use crate::postgresql::CONNECTION_TIMEOUT;
use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tracing::{debug, info};

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
        debug!("[frontend] read");

        let mut message =
            timeout(CONNECTION_TIMEOUT, protocol::read_message(&mut self.client)).await??;

        debug!("message: {:?}", message);

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

impl From<Code> for u8 {
    fn from(code: Code) -> Self {
        match code {
            Code::Bind => b'B',
            Code::Parse => b'P',
            Code::Query => b'Q',
            Code::Unknown(c) => c as u8,
        }
    }
}
