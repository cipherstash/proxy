use super::messages::authentication::Authentication;
use crate::{
    error::{Error, ProtocolError},
    log::PROTOCOL,
    SIZE_I32, SIZE_U8,
};
use bytes::{BufMut, BytesMut};
use pg_proto::codec::{
    Backend, BackendMessage, Direction, Frontend, FrontendMessage, PgCodec, DEFAULT_MAX_FRAME_LEN,
};
use std::{
    io::{BufRead, Cursor},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    time::timeout,
};
use tokio_util::codec::Decoder;
use tokio_util::codec::Encoder;
use tracing::{debug, error};

type Code = u8;

pub fn decode_frontend_frame(bytes: &BytesMut) -> Result<FrontendMessage, Error> {
    let mut bytes = bytes.clone();
    PgCodec::<Frontend>::default()
        .decode(&mut bytes)?
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "partial frontend frame").into()
        })
}

pub fn decode_backend_frame(bytes: &BytesMut) -> Result<BackendMessage, Error> {
    let mut bytes = bytes.clone();
    PgCodec::<Backend>::default()
        .decode(&mut bytes)?
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "partial backend frame").into()
        })
}

pub fn encode_frontend_message(message: &FrontendMessage) -> Result<BytesMut, Error> {
    let mut bytes = BytesMut::new();
    PgCodec::<Frontend>::default().encode(message.to_frame()?, &mut bytes)?;
    Ok(bytes)
}

pub fn encode_backend_message(message: &BackendMessage) -> Result<BytesMut, Error> {
    let mut bytes = BytesMut::new();
    PgCodec::<Backend>::default().encode(message.to_frame()?, &mut bytes)?;
    Ok(bytes)
}

#[derive(Clone, Debug, PartialEq)]
pub enum StartupCode {
    ProtocolVersionNumber,
    CancelRequest,
    SSLRequest,
    GSSENCRequest,
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
    client_id: i32,
) -> Result<Authentication, Error> {
    let connection_timeout = Duration::from_millis(1000 * 10);
    let (_code, bytes, _message) =
        read_backend_message_with_timeout(&mut stream, client_id, connection_timeout).await?;
    Authentication::try_from(&bytes)
}

///
/// Reads a Postgres message from client with an optional timeout
///
/// Timeout values are in config
///
///
pub async fn read_frontend_message<S: AsyncRead + Unpin>(
    mut stream: S,
    client_id: i32,
    connection_timeout: Option<Duration>,
) -> Result<(Code, BytesMut, FrontendMessage), Error> {
    match connection_timeout {
        Some(duration) => read_frontend_message_with_timeout(stream, client_id, duration).await,
        None => read::<Frontend, _>(&mut stream, client_id).await,
    }
}

pub async fn read_backend_message<S: AsyncRead + Unpin>(
    mut stream: S,
    client_id: i32,
    connection_timeout: Option<Duration>,
) -> Result<(Code, BytesMut, BackendMessage), Error> {
    match connection_timeout {
        Some(duration) => read_backend_message_with_timeout(stream, client_id, duration).await,
        None => read::<Backend, _>(&mut stream, client_id).await,
    }
}

///
/// Reads a Postgres message from client with a timeout
///
/// Timeout values are in config
///
///
async fn read_frontend_message_with_timeout<S: AsyncRead + Unpin>(
    mut stream: S,
    client_id: i32,
    duration: Duration,
) -> Result<(Code, BytesMut, FrontendMessage), Error> {
    timeout(duration, read::<Frontend, _>(&mut stream, client_id))
        .await
        .map_err(|_| Error::ConnectionTimeout { duration })?
}

async fn read_backend_message_with_timeout<S: AsyncRead + Unpin>(
    mut stream: S,
    client_id: i32,
    duration: Duration,
) -> Result<(Code, BytesMut, BackendMessage), Error> {
    timeout(duration, read::<Backend, _>(&mut stream, client_id))
        .await
        .map_err(|_| Error::ConnectionTimeout { duration })?
}

///
/// Reads a Postgres message from client
///
/// The SSLRequest/Response sequence requires the Backend to inspect the first byte of the message
/// Byte is then passed as `code` to this function to preserve the message structure
///
///
async fn read<D: Direction, S: AsyncRead + Unpin>(
    mut stream: S,
    client_id: i32,
) -> Result<(Code, BytesMut, D::Message), Error> {
    let code = stream.read_u8().await?;
    let len = stream.read_i32().await?;

    // Detect unexpected message len and avoid panic on read_exact
    // Len must be at least 4 bytes (4 bytes for len/i32)
    if len < SIZE_I32 as i32 || len as usize + SIZE_U8 > DEFAULT_MAX_FRAME_LEN {
        error!(
            msg = "Unexpected PostgreSQL message length",
            code = code,
            len = len
        );
        return Err(ProtocolError::UnexpectedMessageLength {
            code,
            len: len.max(0) as usize,
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

    // Direction-specific pg-proto decoding validates both the frame and message body.
    // In particular, unknown tags fail closed instead of being passed through.
    let mut validated = bytes.clone();
    let message = PgCodec::<D>::default()
        .decode(&mut validated)?
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "partial PostgreSQL frame",
            )
        })?;

    debug!(target: PROTOCOL, client_id, code = ?(code as char), ?bytes);

    Ok((code, bytes, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_proto::codec::{Authentication as PgAuthentication, BackendMessage};
    use tokio::io::{duplex, AsyncWriteExt};
    use tokio_util::codec::Encoder;

    fn encode_backend(message: BackendMessage) -> BytesMut {
        let mut bytes = BytesMut::new();
        PgCodec::<Backend>::default()
            .encode(message.to_frame().unwrap(), &mut bytes)
            .unwrap();
        bytes
    }

    #[tokio::test]
    async fn frontend_frame_can_arrive_in_partial_writes() {
        let (mut writer, mut reader) = duplex(64);
        let task = tokio::spawn(async move {
            writer.write_all(b"Q\0\0").await.unwrap();
            writer.write_all(b"\0\x0dselect 1\0").await.unwrap();
        });

        let (tag, bytes, _) = read_frontend_message(&mut reader, 1, None).await.unwrap();
        assert_eq!(tag, b'Q');
        assert_eq!(&bytes[..], b"Q\0\0\0\x0dselect 1\0");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn unknown_frontend_tag_is_rejected() {
        let (mut writer, mut reader) = duplex(16);
        writer.write_all(b"?\0\0\0\x04").await.unwrap();

        let error = read_frontend_message(&mut reader, 1, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unknown frontend message tag"));
    }

    #[tokio::test]
    async fn malformed_and_oversized_frames_are_rejected_before_body_allocation() {
        let (mut writer, mut reader) = duplex(16);
        writer.write_all(b"Q\0\0\0\x03").await.unwrap();
        assert!(read_frontend_message(&mut reader, 1, None).await.is_err());

        let (mut writer, mut reader) = duplex(16);
        let oversized = (DEFAULT_MAX_FRAME_LEN as u32).to_be_bytes();
        writer.write_all(b"Q").await.unwrap();
        writer.write_all(&oversized).await.unwrap();
        assert!(read_frontend_message(&mut reader, 1, None).await.is_err());
    }

    #[tokio::test]
    async fn authentication_modes_are_validated_by_the_backend_codec() {
        let messages = [
            PgAuthentication::Ok,
            PgAuthentication::CleartextPassword,
            PgAuthentication::Md5Password { salt: *b"salt" },
            PgAuthentication::Sasl {
                mechanisms: vec![bytes::Bytes::from_static(b"SCRAM-SHA-256")],
            },
        ];

        for authentication in messages {
            let (mut writer, mut reader) = duplex(128);
            writer
                .write_all(&encode_backend(BackendMessage::Authentication(
                    authentication,
                )))
                .await
                .unwrap();
            read_auth_message(&mut reader, 1).await.unwrap();
        }
    }
}
