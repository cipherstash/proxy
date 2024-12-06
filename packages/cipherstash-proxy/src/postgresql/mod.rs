mod backend;
mod context;
mod format_code;
mod frontend;
mod messages;
mod protocol;
mod startup;

use crate::{connect::AsyncStream, encrypt::Encrypt, error::Error, tls};
use backend::Backend;
use frontend::Frontend;
use protocol::StartupCode;
use std::time::Duration;
use tokio::io::{split, AsyncWriteExt};
use tracing::info;

pub const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1000 * 1 * 10);

pub const PROTOCOL_VERSION_NUMBER: i32 = 196608;

pub const SSL_REQUEST: i32 = 80877103;

pub const CANCEL_REQUEST: i32 = 80877102;

pub const SSL_RESPONSE_NO: u8 = b'N';

pub const SSL_RESPONSE_YES: u8 = b'S';

///
///
/// Entry point for handling postgres protocol connections
/// Each inbound client connection is mapped to a database connection
/// Hilarity ensues
///
/// Startup flow
///
///     Connect to database with TLS if required
///     First message is either:
///         - SSLRequest
///         - ProtocolVersionNumber
///         - CancelRequest
///
///     On SSLRequest
///         Send SSLResponse
///         Connect with TLS if configured
///
///         On TLS Connect
///             Expect message containing ProtocolVersionNumber is sent
///
///     On CancelRequest
///         Propagate and disconnect
///
///     On ProtocolVersionNumber
///         Propagate and continue
///
///
pub async fn handle(client_stream: AsyncStream, encrypt: Encrypt) -> Result<(), Error> {
    let mut client_stream = client_stream;

    // Connect to the database server, using TLS if configured
    let stream = AsyncStream::connect(&encrypt.config.database.to_socket_address()).await?;
    let mut database_stream = startup::with_tls(stream, &encrypt).await?;
    info!(
        database = encrypt.config.database.to_socket_address(),
        "Database connected"
    );

    loop {
        let startup_message = startup::read_message(&mut client_stream).await?;

        match &startup_message.code {
            StartupCode::SSLRequest => {
                startup::send_ssl_response(&encrypt, &mut client_stream).await?;
                if let Some(ref tls) = encrypt.config.tls {
                    match client_stream {
                        AsyncStream::Tcp(stream) => {
                            // The Client is connecting to our Server
                            let tls_stream = tls::server(stream, tls).await?;
                            client_stream = AsyncStream::Tls(tls_stream);
                        }
                        AsyncStream::Tls(_) => {
                            unreachable!();
                        }
                    }
                }
            }
            StartupCode::CancelRequest => {
                database_stream.write_all(&startup_message.bytes).await?;
                return Err(Error::CancelRequest);
            }
            StartupCode::ProtocolVersionNumber => {
                database_stream.write_all(&startup_message.bytes).await?;
                break;
            }
        }
    }

    let (client_reader, client_writer) = split(client_stream);
    let (server_reader, server_writer) = split(database_stream);

    let mut frontend = Frontend::new(client_reader, server_writer, encrypt.clone());
    let mut backend = Backend::new(client_writer, server_reader, encrypt.clone());

    let client_to_server = async {
        loop {
            frontend.rewrite().await?;
        }
        // Unreachable, but helps the compiler understand the return type
        // TODO: extract into a function or something with type
        Ok::<(), Error>(())
    };

    let server_to_client = async {
        loop {
            backend.rewrite().await?;
        }
        Ok::<(), Error>(())
    };

    // Direct connections, can be handy for debugging
    // let client_to_server = async {
    //     io::copy(&mut client_reader, &mut server_writer).await?;
    //     Ok::<(), Error>(())
    // };
    // let server_to_client = async {
    //     io::copy(&mut server_reader, &mut client_writer).await?;
    //     Ok::<(), Error>(())
    // };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}
