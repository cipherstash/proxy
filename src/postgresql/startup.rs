use std::sync::Arc;

use bytes::{BufMut, BytesMut};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{server::TlsStream, TlsAcceptor, TlsConnector};
use tracing::{debug, error, info};

use crate::{
    config::TlsConfig,
    encrypt::Encrypt,
    error::{Error, ProtocolError},
    postgresql::{PROTOCOL_VERSION_NUMBER, SSL_REQUEST, SSL_RESPONSE_NO, SSL_RESPONSE_YES},
    tcp::{self, AsyncStream},
    tls::NoCertificateVerification,
};

// pub async fn accept_tls<'a>(encrypt: &Encrypt, stream: &mut TcpStream) -> Result<T, Error> {
//     debug!("accept_tls");
//     let s = match encrypt.config.tls {
//         Some(ref tls) => {
//             let server_config = tls.server_config()?;
//             let acceptor = TlsAcceptor::from(Arc::new(server_config));
//             let tls_stream = acceptor.accept(stream).await?;
//             debug!("Client TLS negotiation complete");
//             tls_stream.into_inner().0
//         }
//         None => stream,
//     };
//     Ok(s)
// }

pub async fn accept_tls(tls: &TlsConfig, stream: TcpStream) -> Result<AsyncStream, Error> {
    debug!("accept_tls");

    let server_config = tls.server_config()?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    let tls_stream = acceptor.accept(stream).await?;

    let stream = AsyncStream::Tls(tls_stream.into());
    Ok(stream)
}

pub async fn connect_database_with_tls(encrypt: &Encrypt) -> Result<AsyncStream, Error> {
    let listen = &encrypt.config.database.to_socket_address();
    // let mut server_stream = tcp::connect_with_retry(&listen).await;
    let mut database_stream = TcpStream::connect(&listen).await?;

    tcp::configure(&mut database_stream);

    if encrypt.config.server.skip_tls() {
        debug!("Skip database TLS connection");
        return Ok(AsyncStream::Tcp(database_stream));
    }

    let server_ssl = send_ssl_request(&mut database_stream).await?;

    if !server_ssl {
        error!("Database cannot connect over TLS");
        return Err(ProtocolError::UnexpectedSSLResponse.into());
    }

    let mut root_cert_store = rustls::RootCertStore::empty();
    root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();

    let mut dangerous = config.dangerous();
    dangerous.set_certificate_verifier(Arc::new(NoCertificateVerification {}));

    info!("Connecting to database over TLS");

    let connector = TlsConnector::from(Arc::new(config));

    let domain = encrypt.config.server.server_name()?.to_owned();
    let tls_stream = connector.connect(domain, database_stream).await?;
    return Ok(AsyncStream::Tls(tls_stream.into()));
}

///
/// Send SSLRequest to the stream and return the response
/// Returns true if the server indicates support for TLS
///
async fn send_ssl_request<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
) -> Result<bool, Error> {
    debug!("Send SSLRequest");
    let mut bytes = BytesMut::with_capacity(12);
    bytes.put_i32(8);
    bytes.put_i32(SSL_REQUEST);

    stream.write_all(&bytes).await?;

    debug!("Wait for SSL Response");

    // Server supports TLS
    match stream.read_u8().await? {
        SSL_RESPONSE_YES => Ok(true),
        SSL_RESPONSE_NO => Ok(false),
        code => {
            error!("Unexpected startup message: {}", code as char);
            return Err(ProtocolError::UnexpectedStartupMessage.into());
        }
    }
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

    debug!("Send SSLResponse: {}", response as char);
    stream.write_all(&[response]).await?;

    Ok(())
}

/// Send the startup packet the server. We're pretending we're a Pg client.
/// This tells the server which user we are and what database we want.
pub async fn send_startup<S>(stream: &mut S, username: &str, database: &str) -> Result<(), Error>
where
    S: AsyncWrite + Unpin,
{
    info!("send_startup {username}/{database}");

    let mut bytes = BytesMut::with_capacity(25);
    bytes.put_i32(PROTOCOL_VERSION_NUMBER);

    bytes.put(&b"user\0"[..]);
    bytes.put_slice(username.as_bytes());
    bytes.put_u8(0);

    // Application name
    bytes.put(&b"application_name\0"[..]);
    bytes.put_slice(&b"my-little-proxy\0"[..]);

    // Database
    bytes.put(&b"database\0"[..]);
    bytes.put_slice(database.as_bytes());
    bytes.put_u8(0);
    bytes.put_u8(0); // Null terminator

    let len = bytes.len() as i32 + 4i32;

    let mut startup = BytesMut::with_capacity(len as usize);

    startup.put_i32(len);
    startup.put(bytes);

    stream.write_all(&startup).await?;

    info!("send_startup complete");

    Ok(())
}
