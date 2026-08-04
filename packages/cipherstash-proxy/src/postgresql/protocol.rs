use crate::error::{Error, ProtocolError};
use bytes::BytesMut;
use pg_proto::codec::{
    Authentication, Backend, BackendMessage, Frontend, FrontendMessage, PgCodec,
};
use pg_proto::transport::Buffered;
use std::time::Duration;
use tokio::{io::AsyncRead, time::timeout};
use tokio_util::codec::Decoder;
use tokio_util::codec::Encoder;

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

///
/// Reads an Auth Message from Stream
///
/// Does not use the default connection timeout as the auth message is expected to be sent immediately
/// 10 seconds is a reasonable timeout for the auth message
///
///
pub async fn read_auth_message<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Backend>,
) -> Result<Authentication, Error> {
    let connection_timeout = Duration::from_millis(1000 * 10);
    let (_bytes, message) = read_backend_message_with_timeout(stream, connection_timeout).await?;
    match message {
        BackendMessage::Authentication(authentication) => Ok(authentication),
        _ => Err(ProtocolError::UnexpectedAuthenticationResponse {
            expected: "Authentication".into(),
            received: -1,
        }
        .into()),
    }
}

///
/// Reads a Postgres message from client with an optional timeout
///
/// Timeout values are in config
///
///
pub async fn read_frontend_message<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Frontend>,
    connection_timeout: Option<Duration>,
) -> Result<(BytesMut, FrontendMessage), Error> {
    match connection_timeout {
        Some(duration) => read_frontend_message_with_timeout(stream, duration).await,
        None => read_frontend(stream).await,
    }
}

pub async fn read_backend_message<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Backend>,
    connection_timeout: Option<Duration>,
) -> Result<(BytesMut, BackendMessage), Error> {
    match connection_timeout {
        Some(duration) => read_backend_message_with_timeout(stream, duration).await,
        None => read_backend(stream).await,
    }
}

///
/// Reads a Postgres message from client with a timeout
///
/// Timeout values are in config
///
///
async fn read_frontend_message_with_timeout<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Frontend>,
    duration: Duration,
) -> Result<(BytesMut, FrontendMessage), Error> {
    timeout(duration, read_frontend(stream))
        .await
        .map_err(|_| Error::ConnectionTimeout { duration })?
}

async fn read_backend_message_with_timeout<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Backend>,
    duration: Duration,
) -> Result<(BytesMut, BackendMessage), Error> {
    timeout(duration, read_backend(stream))
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
async fn read_frontend<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Frontend>,
) -> Result<(BytesMut, FrontendMessage), Error> {
    let message = stream.receive_wire().await?;
    let bytes = encode_frontend_message(&message)?;
    Ok((bytes, message))
}

async fn read_backend<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Backend>,
) -> Result<(BytesMut, BackendMessage), Error> {
    let message = stream.receive_backend().await?;
    let bytes = encode_backend_message(&message)?;
    Ok((bytes, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_proto::codec::{
        Authentication as PgAuthentication, BackendMessage, DEFAULT_MAX_FRAME_LEN,
    };
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
        let (mut writer, reader) = duplex(64);
        let task = tokio::spawn(async move {
            writer.write_all(b"Q\0\0").await.unwrap();
            writer.write_all(b"\0\x0dselect 1\0").await.unwrap();
        });
        let mut reader = Buffered::new_frontend(reader);

        let (bytes, _) = read_frontend_message(&mut reader, None).await.unwrap();
        assert_eq!(&bytes[..], b"Q\0\0\0\x0dselect 1\0");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn unknown_frontend_tag_is_rejected() {
        let (mut writer, reader) = duplex(16);
        writer.write_all(b"?\0\0\0\x04").await.unwrap();
        let mut reader = Buffered::new_frontend(reader);

        let error = read_frontend_message(&mut reader, None).await.unwrap_err();
        assert!(error.to_string().contains("unknown frontend message tag"));
    }

    #[tokio::test]
    async fn malformed_and_oversized_frames_are_rejected_before_body_allocation() {
        let (mut writer, reader) = duplex(16);
        writer.write_all(b"Q\0\0\0\x03").await.unwrap();
        let mut reader = Buffered::new_frontend(reader);
        assert!(read_frontend_message(&mut reader, None).await.is_err());

        let (mut writer, reader) = duplex(16);
        let oversized = (DEFAULT_MAX_FRAME_LEN as u32).to_be_bytes();
        writer.write_all(b"Q").await.unwrap();
        writer.write_all(&oversized).await.unwrap();
        let mut reader = Buffered::new_frontend(reader);
        assert!(read_frontend_message(&mut reader, None).await.is_err());
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
            let (mut writer, reader) = duplex(128);
            writer
                .write_all(&encode_backend(BackendMessage::Authentication(
                    authentication,
                )))
                .await
                .unwrap();
            let mut reader = Buffered::new(reader);
            read_auth_message(&mut reader).await.unwrap();
        }
    }
}
