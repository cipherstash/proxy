use crate::{
    error::{Error, ProtocolError},
    postgresql::PROTOCOL_VERSION_NUMBER,
    SIZE_I32, SIZE_U8,
};
use bytes::{BufMut, BytesMut};
use std::io::{BufRead, Cursor};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::TcpStream,
};
use tracing::{debug, error};

use super::{CANCEL_REQUEST, SSL_REQUEST};

#[derive(Clone, Debug)]
pub enum StartupCode {
    ProtocolVersionNumber,
    CancelRequest,
    SSLRequest,
}

#[derive(Clone, Debug)]
pub struct StartupMessage {
    pub code: StartupCode,
    pub bytes: BytesMut,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub code: u8,
    pub bytes: BytesMut,
}

impl From<i32> for StartupCode {
    fn from(code: i32) -> Self {
        match code {
            PROTOCOL_VERSION_NUMBER => StartupCode::ProtocolVersionNumber,
            SSL_REQUEST => StartupCode::SSLRequest,
            CANCEL_REQUEST => StartupCode::CancelRequest,
            _ => panic!("Unexpected startup code {code}"),
        }
    }
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

// pub async fn ssl_request(mut client: TcpStream) -> Result<bool, Error> {
//     Ok(true)
// }

///
/// Read the start up message from the client
/// Startup messages are sent by the client to the server to initiate a connection
///
///
///
pub async fn read_startup_message<C>(client: &mut C) -> Result<StartupMessage, Error>
where
    C: AsyncRead + Unpin,
{
    let len = client.read_i32().await?;
    debug!("[read_start_up_message]");

    let capacity = len as usize;

    let mut bytes = BytesMut::with_capacity(capacity);
    bytes.put_i32(len);
    bytes.resize(capacity, b'0');

    let slice_start = SIZE_I32;
    client.read_exact(&mut bytes[slice_start..]).await?;

    // code is the first 4 bytes after len
    let code_bytes: [u8; 4] = [
        bytes.as_ref()[4],
        bytes.as_ref()[5],
        bytes.as_ref()[6],
        bytes.as_ref()[7],
    ];

    let code = i32::from_be_bytes(code_bytes);

    Ok(StartupMessage {
        code: code.into(),
        bytes,
    })
}

///
/// Reads a Postgres message from client
///
/// The SSLRequest/Response sequence requires the Backend to inspect the first byte of the message
/// Byte is then passed as `code` to this function to preserve the message structure
///
///
pub async fn read_message<C: AsyncRead + Unpin>(mut client: C) -> Result<Message, Error> {
    let code = client.read_u8().await?;
    debug!("[read_message] code: {}", code as char);
    let len = client.read_i32().await?;
    debug!("[read_message] len: {len}");
    // debug!("[read_message] code: {code}, len: {len}");

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
