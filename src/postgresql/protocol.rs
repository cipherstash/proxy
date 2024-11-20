use crate::{
    error::{Error, ProtocolError},
    SIZE_I32, SIZE_U8,
};
use bytes::{BufMut, BytesMut};
use std::io::{BufRead, Cursor};
use tokio::io::{AsyncRead, AsyncReadExt};
use tracing::{debug, error};

#[derive(Clone, Debug)]
pub struct Message {
    pub code: u8,
    pub bytes: BytesMut,
}

pub trait BytesMutReadString {
    fn read_string(&mut self) -> Result<String, Error>;
}

impl BytesMutReadString for Cursor<&BytesMut> {
    /// Should only be used when reading strings from the message protocol.
    /// Can be used to read multiple strings from the same message which are separated by the null byte
    fn read_string(&mut self) -> Result<String, Error> {
        let mut buf = Vec::with_capacity(512);
        match self.read_until(b'\0', &mut buf) {
            Ok(_) => Ok(String::from_utf8_lossy(&buf[..buf.len() - 1]).to_string()),
            Err(err) => Err(err.into()),
        }
    }
}
///
/// Reads a Postgres message from client
///
/// The SSLRequest/Response sequence requires the Backend to inspect the first byte of the message
/// Byte is then passed as `code` to this function to preserve the message structure
///
///
pub async fn read_message<C: AsyncRead + Unpin>(mut client: C, code: u8) -> Result<Message, Error> {
    debug!("[read_message]");

    let len = client.read_i32().await?;

    // Detect unexpected message len and avoid panic on read_exact
    // Len must be at least 4 bytes (4 bytes for len/i32)
    if (len as usize) < SIZE_I32 {
        error!(code = code, len = len, "Unexpected message length");
        return Err(ProtocolError::UnexpectedMessageLength {
            code,
            len: len as usize,
        }
        .into());
    }

    let capacity = len as usize + SIZE_U8; //len plus len of code
    let mut bytes = BytesMut::with_capacity(capacity);

    bytes.put_u8(code);
    bytes.put_i32(len);

    let slice_start = bytes.len();

    // Capacity and len are not the same!!
    // resize populates the buffer with 0s
    bytes.resize(capacity, b'0');

    client.read_exact(&mut bytes[slice_start..]).await?;

    let message = Message { code, bytes };

    Ok(message)
}

pub async fn read_message_with_code<C: AsyncRead + Unpin>(mut client: C) -> Result<Message, Error> {
    let code = client.read_u8().await?;
    read_message(client, code).await
}
