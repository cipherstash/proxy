mod backend;
mod bind;
mod format_code;
mod frontend;
mod parse;
mod protocol;
mod query;
mod startup;

use crate::{connect::AsyncStream, encrypt::Encrypt, error::Error, tls};
use backend::Backend;
use frontend::Frontend;
use protocol::StartupCode;
use std::time::Duration;
use tokio::io::{split, AsyncWriteExt};
use tracing::{debug, info};

pub const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1000 * 1 * 10);

pub const PROTOCOL_VERSION_NUMBER: i32 = 196608;

pub const SSL_REQUEST: i32 = 80877103;

pub const CANCEL_REQUEST: i32 = 80877102;

/// Protocol message codes.
const BIND: u8 = b'B';
const PARSE: u8 = b'P';
const QUERY: u8 = b'Q';
const NULL: i32 = -1;

const SSL_RESPONSE_YES: u8 = b'S';
const SSL_RESPONSE_NO: u8 = b'N';

///
/// TODO This needs to be abstracted once design stabilises
///
/// Keeping it here for now
///  - mostly fits in my head so rapid iteration easier
///  - the protocol flow and interaction with TLS is a bit wacky and  I am unsure of the target structure
///
/// async fn handle<T: AsyncRead + AsyncWrite + Unpin>(
pub async fn handle(client_stream: AsyncStream, encrypt: Encrypt) -> Result<(), Error> {
    let mut client_stream = client_stream;

    // Connect to the database server, using TLS if configured
    let stream = AsyncStream::connect(&encrypt.config.database.to_socket_address()).await?;
    let mut database_stream = startup::to_tls(stream, &encrypt).await?;
    info!(
        database = encrypt.config.database.to_socket_address(),
        tls = encrypt.config.server.use_tls,
        "Database connected"
    );

    // This is the client loop
    // The database will already be connected with TLS if required
    // We do not need to propagate the SSLRequest to the database
    loop {
        let startup_message = startup::read_message(&mut client_stream).await?;
        info!("startup_message {:?}", startup_message);
        match &startup_message.code {
            StartupCode::SSLRequest => {
                debug!("SSLRequest");
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
            StartupCode::ProtocolVersionNumber => {
                debug!("ProtocolVersionNumber");
                database_stream.write_all(&startup_message.bytes).await?;
                break;
            }
            StartupCode::CancelRequest => {
                debug!("CancelRequest");
                // propagate the cancel request to the server and end the connection
                database_stream.write_all(&startup_message.bytes).await?;
                return Err(Error::CancelRequest);
            }
        }
    }

    let (client_reader, client_writer) = split(client_stream);
    let (server_reader, server_writer) = split(database_stream);

    let mut frontend = Frontend::new(client_reader, server_writer, encrypt.clone());
    let mut backend = Backend::new(client_writer, server_reader, encrypt.clone());

    let client_to_server = async {
        loop {
            let bytes = frontend.read().await?;
            frontend.write(bytes).await?; // debug!("write complete");
        }

        // Unreachable, but helps the compiler understand the return type
        // TODO: extract into a function
        Ok::<(), Error>(())
    };

    let server_to_client = async {
        loop {
            let bytes = backend.read().await?;
            backend.write(bytes).await?;
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
