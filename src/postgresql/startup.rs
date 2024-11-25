use bytes::{BufMut, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, info, warn};

use crate::{
    connect::AsyncStream,
    encrypt::Encrypt,
    error::{Error, ProtocolError},
    postgresql::{PROTOCOL_VERSION_NUMBER, SSL_REQUEST, SSL_RESPONSE_NO, SSL_RESPONSE_YES},
    tls, SIZE_I32,
};

use super::protocol::StartupMessage;

pub async fn to_tls(stream: AsyncStream, encrypt: &Encrypt) -> Result<AsyncStream, Error> {
    if encrypt.config.server.skip_tls() {
        debug!("Skip database TLS connection");
        return Ok(stream);
    }

    match stream {
        AsyncStream::Tcp(mut tcp_stream) => {
            let server_ssl = send_ssl_request(&mut tcp_stream).await?;

            if !server_ssl {
                error!("Database cannot connect over TLS");
                return Err(ProtocolError::UnexpectedSSLResponse.into());
            }

            let tls_stream = tls::client(tcp_stream, encrypt).await?;
            Ok(AsyncStream::Tls(tls_stream.into()))
        }
        AsyncStream::Tls(_) => {
            warn!("Database already connected over TLS");
            Ok(stream)
        }
    }
}

///
/// Read the start up message from the client
/// Startup messages are sent by the client to the server to initiate a connection
///
///
///
pub async fn read_message<C>(client: &mut C) -> Result<StartupMessage, Error>
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

// pub struct Startup {
//     encrypt: Encrypt,
// }

// impl Startup {
//     pub fn new(encrypt: Encrypt) -> Self {
//         Self { encrypt }
//     }

//     pub async fn connect_database(&self) -> Result<AsyncStream, Error> {
//         let stream = connect_database_with_tls(&self.encrypt).await?;
//         Ok(stream)
//     }

//     // pub async fn handle(&mut self) -> Result<Self, Error> {
//     //     // This is the client loop
//     //     // The database will already be connected with TLS if required
//     //     // We do not need to propagate the SSLRequest to the database
//     //     loop {
//     //         // let mut client_stream = self.client_stream.borrow_mut();
//     //         let startup_message = read_startup_message(&mut *client_stream).await?;
//     //         info!("startup_message {:?}", startup_message);
//     //         match &startup_message.code {
//     //             StartupCode::SSLRequest => {
//     //                 debug!("SSLRequest");
//     //                 send_ssl_response(&self.encrypt, &mut *client_stream).await?;
//     //                 if let Some(tls) = &self.encrypt.config.tls {
//     //                     let mut client_stream = self.client_stream.borrow_mut();
//     //                     match &mut *client_stream {
//     //                         AsyncStream::Tcp(stream) => {
//     //                             let server_config = tls.server_config()?;
//     //                             let acceptor = TlsAcceptor::from(Arc::new(server_config));

//     //                             let tls = acceptor.accept(stream).await?;
//     //                             let stream = AsyncStream::Tls(tls);
//     //                         }
//     //                         AsyncStream::Tls(_) => {
//     //                             unreachable!();
//     //                         }
//     //                     }
//     //                 }
//     //             }
//     //             StartupCode::ProtocolVersionNumber => {
//     //                 debug!("ProtocolVersionNumber");
//     //                 debug!("{:?}", &startup_message.bytes);
//     //                 self.database_stream
//     //                     .write_all(&startup_message.bytes)
//     //                     .await?;
//     //                 break;
//     //             }
//     //             StartupCode::CancelRequest => {
//     //                 debug!("CancelRequest");
//     //                 // propagate the cancel request to the server and end the connection
//     //                 self.database_stream
//     //                     .write_all(&startup_message.bytes)
//     //                     .await?;
//     //                 return Err(Error::CancelRequest);
//     //             }
//     //         }
//     //     }
//     //     // Ok(Self {
//     //     //     encrypt: self.encrypt,
//     //     //     client_stream: self.client_stream,
//     //     //     database_stream: self.database_stream,
//     //     // })
//     // }

//     // pub async fn split(
//     //     self,
//     // ) -> Result<
//     //     (
//     //         Frontend<ReadHalf<AsyncStream>, WriteHalf<AsyncStream>>,
//     //         Backend<WriteHalf<AsyncStream>, ReadHalf<AsyncStream>>,
//     //     ),
//     //     Error,
//     // > {
//     //     let (client_reader, client_writer) = self.client_stream.split().await;
//     //     let (server_reader, server_writer) = self.database_stream.split().await;

//     //     let fe = Frontend::new(client_reader, server_writer, self.encrypt.clone());
//     //     let be = Backend::new(client_writer, server_reader, self.encrypt.clone());

//     //     Ok((fe, be))
//     // }
// }
