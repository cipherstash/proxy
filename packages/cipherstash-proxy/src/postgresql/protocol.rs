use super::{messages::authentication::Authentication, CANCEL_REQUEST, SSL_REQUEST};
use crate::{
    error::{Error, ProtocolError},
    log::PROTOCOL,
    postgresql::PROTOCOL_VERSION_NUMBER,
    SIZE_I32, SIZE_U8,
};
use bytes::{BufMut, BytesMut};
use std::{
    io::{BufRead, Cursor},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    time::timeout,
};
use tracing::{debug, error};

#[derive(Clone, Debug, PartialEq)]
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

///
/// Reads an Auth Message from Stream
///
/// Does not use the default connection timeout as the auth message is expected to be sent immediately
/// 10 seconds is a reasonable timeout for the auth message
///
///
pub async fn read_auth_message<S: AsyncRead + Unpin>(
    mut stream: S,
) -> Result<Authentication, Error> {
    let connection_timeout = Duration::from_millis(1000 * 10);
    let message = read_message_with_timeout(&mut stream, connection_timeout).await?;
    Authentication::try_from(&message.bytes)
}

pub async fn read_message_with_timeout<S: AsyncRead + Unpin>(
    mut stream: S,
    connection_timeout: Duration,
) -> Result<Message, Error> {
    timeout(connection_timeout, read_message(&mut stream)).await?
}

///
/// Reads a Postgres message from client
///
/// The SSLRequest/Response sequence requires the Backend to inspect the first byte of the message
/// Byte is then passed as `code` to this function to preserve the message structure
///
///
pub async fn read_message<S: AsyncRead + Unpin>(mut stream: S) -> Result<Message, Error> {
    let code = stream.read_u8().await?;
    // debug!("[read_message] code: {}", code as char);
    let len = stream.read_i32().await?;
    // debug!("[read_message] len: {len}");
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
    bytes.resize(capacity, 0);

    stream.read_exact(&mut bytes[slice_start..]).await?;

    let message = Message { code, bytes };

    debug!(target: PROTOCOL, "{message:?}");

    Ok(message)
}
