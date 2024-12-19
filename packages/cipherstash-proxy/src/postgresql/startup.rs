use std::time::Duration;

use bytes::{BufMut, BytesMut};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};
use tracing::{debug, error, warn};

use crate::{
    connect::AsyncStream,
    encrypt::Encrypt,
    error::{Error, ProtocolError},
    log::{DEVELOPMENT, PROTOCOL},
    postgresql::{SSL_REQUEST, SSL_RESPONSE_NO, SSL_RESPONSE_YES},
    tls, TandemConfig, SIZE_I32,
};

use super::protocol::StartupMessage;

pub async fn with_tls(stream: AsyncStream, config: &TandemConfig) -> Result<AsyncStream, Error> {
    if config.disable_database_tls() {
        warn!(
            DEVELOPMENT,
            "Connecting to database without Transport Layer Security (TLS)"
        );
        return Ok(stream);
    }
    match stream {
        AsyncStream::Tcp(mut tcp_stream) => {
            let server_supports_ssl = send_ssl_request(&mut tcp_stream).await?;

            match server_supports_ssl {
                true => {
                    let tls_stream = tls::client(tcp_stream, config).await?;
                    Ok(AsyncStream::Tls(tls_stream))
                }
                false => {
                    warn!("Connecting to database without Transport Layer Security (TLS)");
                    Ok(AsyncStream::Tcp(tcp_stream))
                }
            }
        }
        AsyncStream::Tls(_) => {
            // Technically unreachable unless the server is misbehaving
            warn!("Database already connected over Transport Layer Security (TLS)");
            Ok(stream)
        }
    }
}

pub async fn read_message_with_timeout<C>(
    client: &mut C,
    connection_timeout: Duration,
) -> Result<StartupMessage, Error>
where
    C: AsyncRead + Unpin,
{
    timeout(connection_timeout, read_message(client)).await?
}

///
/// Read the start up message from the client
/// Startup messages are sent by the client to the server to initiate a connection
///
///
///
async fn read_message<C>(client: &mut C) -> Result<StartupMessage, Error>
where
    C: AsyncRead + Unpin,
{
    let len = client.read_i32().await?;

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

    let message = StartupMessage {
        code: code.into(),
        bytes,
    };
    debug!(target: PROTOCOL, "StartupMessage {:?}", message);

    Ok(message)
}

///
/// Send SSLRequest to the stream and return the response
/// Returns true if the server indicates support for TLS
///
pub async fn send_ssl_request<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
) -> Result<bool, Error> {
    let mut bytes = BytesMut::with_capacity(12);
    bytes.put_i32(8);
    bytes.put_i32(SSL_REQUEST);

    stream.write_all(&bytes).await?;

    // Server supports TLS
    let response = match stream.read_u8().await? {
        SSL_RESPONSE_YES => true,
        SSL_RESPONSE_NO => false,
        code => {
            error!("Unexpected startup message: {}", code as char);
            return Err(ProtocolError::UnexpectedStartupMessage.into());
        }
    };

    debug!(target: PROTOCOL, "Received SSLResponse {response}");
    Ok(response)
}

///
/// Send SSLRequest to the stream
/// Returns true if the server indicates support for TLS
/// N for no, S for yeS or tlS
/// The SSLResponse MUST come before the TLS handshake
///
pub async fn send_ssl_response<T: AsyncWrite + Unpin>(
    encrypt: &Encrypt,
    stream: &mut T,
) -> Result<(), Error> {
    let response = match encrypt.config.tls {
        Some(_) => b'S',
        None => b'N',
    };

    debug!(target: PROTOCOL, "Send SSLResponse {response}");

    stream.write_all(&[response]).await?;

    Ok(())
}
