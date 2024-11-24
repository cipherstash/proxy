use std::sync::Arc;

use bytes::{BufMut, BytesMut};
use my_little_proxy::config::TandemConfig;
use my_little_proxy::encrypt::Encrypt;
use my_little_proxy::error::{Error, ProtocolError};
use my_little_proxy::postgresql::{
    accept_tls, connect_database_with_tls, send_ssl_response, PROTOCOL_VERSION_NUMBER,
};
use my_little_proxy::tcp::AsyncStream;
use my_little_proxy::{postgresql as pg, tcp, trace};
use tokio::io::{self, split, AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, warn};

// TODO: Accept command line arguments for config file path
#[tokio::main]
async fn main() {
    let config_file = "cipherstash-proxy.toml";

    trace();

    let config = match TandemConfig::load(config_file) {
        Ok(config) => config,
        Err(err) => {
            error!("Configuration Error: {}", err);
            std::process::exit(exitcode::CONFIG);
        }
    };
    let encrypt = startup(config).await;
    // let listener = tcp::bind_with_retry(&encrypt.config.server).await;
    let listener = TcpListener::bind(&encrypt.config.server.to_socket_address())
        .await
        .unwrap();

    loop {
        // let (stream, _) = listener.accept().await.unwrap();
        let stream = AsyncStream::accept(&listener).await.unwrap();

        let encrypt = encrypt.clone();

        tokio::spawn(async move {
            let encrypt = encrypt.clone();

            match handle(encrypt, stream).await {
                Ok(_) => (),
                Err(e) => {
                    error!("Error {:?}", e);
                    match e {
                        Error::ConnectionClosed => {
                            info!("Database connection closed by client");
                        }
                        Error::CancelRequest => {
                            info!("Database connection closed after cancel request");
                        }
                        Error::ConnectionTimeout(_) => {
                            warn!("Database connection timeout");
                        }
                        _ => {
                            error!("Error {:?}", e);
                        }
                    }
                }
            }
        });
    }
}

///
/// Validate various configuration options and
/// Init the Encrypt service
///
async fn startup(config: TandemConfig) -> Encrypt {
    if config.encrypt.dataset_id.is_none() {
        info!("Encrypt using default dataset");
    }

    match config.server.server_name() {
        Ok(_) => {}
        Err(err) => {
            error!("{}", err);
            std::process::exit(exitcode::CONFIG);
        }
    }

    if config.server.skip_tls() {
        warn!("Connecting to database without Transport Layer Security (TLS)");
    }

    match &config.tls {
        Some(tls) => {
            if !tls.cert_exists() {
                println!("Certificate not found: {}", tls.certificate);
                std::process::exit(exitcode::CONFIG);
            }

            if !tls.private_key_exists() {
                println!("Private key not found: {}", tls.private_key);
                std::process::exit(exitcode::CONFIG);
            }

            match tls.server_config() {
                Ok(_) => {
                    info!("Transport Layer Security (TLS) configuration validated");
                }
                Err(err) => {
                    error!("Transport Layer Security (TLS) configuration error");
                    error!("{}", err);
                    std::process::exit(exitcode::CONFIG);
                }
            }
        }
        None => {
            warn!("Transport Layer Security (TLS) is not configured");
        }
    }

    match Encrypt::init(config).await {
        Ok(encrypt) => {
            info!("Encrypt connected");
            encrypt
        }
        Err(err) => {
            error!("Encrypt could not connect");
            error!("{}", err);
            std::process::exit(exitcode::UNAVAILABLE);
        }
    }
}

///
/// TODO This needs to be abstracted once design stabilises
///
/// Keeping it here for now
///  - mostly fits in my head so rapid iteration easier
///  - the protocol flow and interaction with TLS is a bit wacky and  I am unsure of the target structure
///
/// async fn handle<T: AsyncRead + AsyncWrite + Unpin>(
async fn handle(encrypt: Encrypt, client_stream: AsyncStream) -> Result<(), Error> {
    warn!("hello");

    let mut client_stream = client_stream;

    // Connect to the database server, using TLS if configured
    let mut database_stream = connect_database_with_tls(&encrypt).await?;
    info!(
        database = encrypt.config.database.to_socket_address(),
        tls = encrypt.config.server.use_tls,
        "Database connected"
    );

    // let mut ssl_request = false;
    // let username = encrypt.config.database.username.as_str();
    // let database = encrypt.config.database.database.as_str();
    // send_startup(&mut server_stream, username, database).await?;
    // if ssl_request {
    //     if startup_message.code != pg::StartupCode::ProtocolVersionNumber {
    //         error!("Expected Startup Message after SSLRequest");
    //         return Err(ProtocolError::UnexpectedSSLRequest.into());
    //     }
    // }

    // This is the client loop
    // The database will already be connected with TLS if required
    // We do not need to propagate the SSLRequest to the database
    loop {
        let startup_message = pg::read_startup_message(&mut client_stream).await?;
        info!("startup_message {:?}", startup_message);
        match &startup_message.code {
            pg::StartupCode::SSLRequest => {
                debug!("SSLRequest");
                send_ssl_response(&encrypt, &mut client_stream).await?;

                if let Some(ref tls) = encrypt.config.tls {
                    // let server_config = tls.server_config()?;
                    // let acceptor = TlsAcceptor::from(Arc::new(server_config));
                    // let mut tls_stream = acceptor.accept(client_stream).await?;

                    match client_stream {
                        AsyncStream::Tcp(stream) => {
                            client_stream = accept_tls(tls, stream).await?;
                        }
                        AsyncStream::Tls(_) => {
                            unreachable!();
                        }
                    }
                }
            }
            pg::StartupCode::ProtocolVersionNumber => {
                debug!("ProtocolVersionNumber");
                debug!("{:?}", &startup_message.bytes);
                database_stream.write_all(&startup_message.bytes).await?;
                break;
            }
            pg::StartupCode::CancelRequest => {
                debug!("CancelRequest");
                // propagate the cancel request to the server and end the connection
                database_stream.write_all(&startup_message.bytes).await?;
                return Err(Error::CancelRequest);
            }
        }
    }

    debug!("==========================================================");

    let (client_reader, mut client_writer) = client_stream.split().await;
    let (server_reader, server_writer) = database_stream.split().await;

    let client_to_server = async {
        let mut fe = pg::Frontend::new(client_reader, server_writer, encrypt);

        loop {
            let bytes = fe.read().await?;
            fe.write(bytes).await?; // debug!("write complete");
        }

        // Unreachable, but helps the compiler understand the return type
        // TODO: extract into a function
        Ok::<(), Error>(())
    };

    let server_to_client = async {
        let mut be = pg::Backend::new(server_reader);

        loop {
            let bytes = be.read().await?;

            client_writer.write_all(&bytes).await?;
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

/// Send the startup packet the server. We're pretending we're a Pg client.
/// This tells the server which user we are and what database we want.
pub async fn send_startup<S>(stream: &mut S, user: &str, database: &str) -> Result<(), Error>
where
    S: AsyncWrite + Unpin,
{
    let mut bytes = BytesMut::with_capacity(25);

    bytes.put_i32(PROTOCOL_VERSION_NUMBER); // Protocol number

    // User
    bytes.put(&b"user\0"[..]);
    bytes.put_slice(user.as_bytes());
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
