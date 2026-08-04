use std::time::Duration;

use pg_proto::{
    codec::Frontend,
    pre_startup::{EncryptionReply, PreStartupMessage},
    transport::Buffered,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};
use tracing::{debug, error, warn};

use crate::{
    connect::AsyncStream,
    error::{Error, ProtocolError},
    log::PROTOCOL,
    tls, TandemConfig,
};

pub async fn with_tls(stream: AsyncStream, config: &TandemConfig) -> Result<AsyncStream, Error> {
    if config.database_tls_disabled() {
        warn!(msg = "Connecting to database without Transport Layer Security (TLS)");
        return Ok(stream);
    }
    match stream {
        AsyncStream::Tcp(mut tcp_stream) => {
            let server_supports_ssl = send_ssl_request(&mut tcp_stream).await?;

            match server_supports_ssl {
                true => {
                    let tls_stream = tls::client(tcp_stream, config).await?;
                    Ok(AsyncStream::Tls(Box::new(tls_stream)))
                }
                false => {
                    warn!(msg = "Connecting to database without Transport Layer Security (TLS)");
                    Ok(AsyncStream::Tcp(tcp_stream))
                }
            }
        }
        AsyncStream::Tls(_) => {
            // Technically unreachable unless the server is misbehaving
            warn!(msg = "Database already connected over Transport Layer Security (TLS)");
            Ok(stream)
        }
    }
}

///
/// Reads a Postgres startup message from client with an optional timeout
///
/// Timeout values are in config
///
///
pub async fn read_message<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Frontend>,
    connection_timeout: Option<Duration>,
) -> Result<PreStartupMessage, Error> {
    match connection_timeout {
        Some(duration) => read_message_with_timeout(stream, duration).await,
        None => read(stream).await,
    }
}

///
/// Reads a Postgres message from client with a timeout
///
/// Timeout values are in config
///
///
async fn read_message_with_timeout<S: AsyncRead + Unpin>(
    stream: &mut Buffered<S, Frontend>,
    duration: Duration,
) -> Result<PreStartupMessage, Error> {
    timeout(duration, read(stream))
        .await
        .map_err(|_| Error::ConnectionTimeout { duration })?
}

///
/// Read the start up message from the client
/// Startup messages are sent by the client to the server to initiate a connection
///
///
///
async fn read<C>(client: &mut Buffered<C, Frontend>) -> Result<PreStartupMessage, Error>
where
    C: AsyncRead + Unpin,
{
    let message = client.receive_pre_startup().await?;
    debug!(target: PROTOCOL, pre_startup = ?message);
    Ok(message)
}

///
/// Send SSLRequest to the stream and return the response
/// Returns true if the server indicates support for TLS
///
pub async fn send_ssl_request<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
) -> Result<bool, Error> {
    stream
        .write_all(&PreStartupMessage::SslRequest.to_packet()?)
        .await?;

    // Server supports TLS
    let response = match EncryptionReply::try_from(stream.read_u8().await?) {
        Ok(EncryptionReply::Accepted) => true,
        Ok(EncryptionReply::Rejected) => false,
        Ok(EncryptionReply::LegacyError) => {
            return Err(ProtocolError::UnexpectedStartupMessage.into());
        }
        Err(err) => {
            let code = err.0;
            error!(msg = "Unexpected startup message", code = ?(code as char));
            return Err(ProtocolError::UnexpectedStartupMessage.into());
        }
    };

    debug!(target: PROTOCOL, msg = "Database SSLResponse", SSLResponse = ?response);
    Ok(response)
}

///
/// Send SSLRequest to the stream
/// Returns true if the server indicates support for TLS
/// N for no, S for yeS or tlS
/// The SSLResponse MUST come before the TLS handshake
///
pub async fn send_ssl_response<T: AsyncWrite + Unpin>(
    stream: &mut T,
    tls: bool,
) -> Result<(), Error> {
    let response = if tls {
        EncryptionReply::Accepted
    } else {
        EncryptionReply::Rejected
    };

    debug!(target: PROTOCOL, msg = "SSLResponse to Client", SSLResponse = ?response);

    stream.write_all(&[response.as_byte()]).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn ssl_and_cancellation_packets_use_pg_proto_pre_startup_decoding() {
        let (mut writer, reader) = duplex(64);
        writer
            .write_all(&PreStartupMessage::SslRequest.to_packet().unwrap())
            .await
            .unwrap();
        let mut reader = Buffered::new_frontend(reader);
        let ssl = read_message(&mut reader, None).await.unwrap();
        assert!(matches!(ssl, PreStartupMessage::SslRequest));

        let cancel = PreStartupMessage::CancelRequest {
            process_id: 42,
            secret_key: bytes::Bytes::from_static(b"key!"),
        }
        .to_packet()
        .unwrap();
        writer.write_all(&cancel).await.unwrap();
        let decoded = read_message(&mut reader, None).await.unwrap();
        assert!(matches!(decoded, PreStartupMessage::CancelRequest { .. }));
        assert_eq!(decoded.to_packet().unwrap(), cancel);
    }

    #[tokio::test]
    async fn ssl_reply_rejects_unknown_bytes() {
        let (mut writer, mut reader) = duplex(8);
        writer.write_all(b"?").await.unwrap();
        assert!(send_ssl_request(&mut reader).await.is_err());

        let (mut client, mut server) = duplex(8);
        send_ssl_response(&mut server, true).await.unwrap();
        let mut response = [0];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(response, [EncryptionReply::Accepted.as_byte()]);
    }
}
