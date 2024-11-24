use bytes::{BufMut, BytesMut};
use my_little_proxy::config::TandemConfig;
use my_little_proxy::encrypt::Encrypt;
use my_little_proxy::error::{Error, ProtocolError};
use my_little_proxy::postgresql::{PROTOCOL_VERSION_NUMBER, SSL_REQUEST};
use my_little_proxy::{postgresql as pg, tcp, tls, trace};
use rustls::client;
use rustls::client::danger::ServerCertVerifier;
use rustls_pki_types::ServerName;
use std::sync::Arc;
use tokio::io::{self, split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time;
use tokio_rustls::server::TlsStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};
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

    let listen = config.server.to_socket_address();

    let encrypt = startup(config).await;

    match handle_tls(encrypt).await {
        Ok(_) => {
            info!("Connection closed");
        }
        Err(e) => {
            error!("Error {:?}", e);
        }
    }
}

// let listener = tcp::bind_with_retry(&listen).await;
// info!(url = listen, "Server connected");

// loop {
//     let (mut stream, _) = listener.accept().await.unwrap();
//     tcp::configure(&mut stream);

//     let encrypt = encrypt.clone();

//     tokio::spawn(async move {
//         loop {
//             let encrypt = encrypt.clone();
//             match handle(encrypt, &mut stream).await {
//                 Ok(_) => (),
//                 Err(e) => {
//                     match e {
//                         Error::ConnectionClosed => {
//                             info!("Database connection closed by client");
//                         }
//                         Error::CancelRequest => {
//                             info!("Database connection closed after cancel request");
//                         }
//                         Error::ConnectionTimeout(_) => {
//                             warn!("Database connection timeout");
//                         }
//                         _ => {
//                             error!("Error {:?}", e);
//                         }
//                     }
//                     break;
//                 }
//             }
//         }
//     });
// }
// }

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
async fn handle<T: AsyncRead + AsyncWrite + Unpin>(
    encrypt: Encrypt,
    mut client_stream: &mut T,
) -> Result<(), Error> {
    let server_stream = connect_server(&encrypt).await?;

    // let mut server_stream = TcpStream::connect(&encrypt.config.connect.to_socket_address()).await?;
    // tcp::configure(&mut server_stream);

    if encrypt.config.server.use_tls {
        // Send SSLRequest to the server
        info!("Send SSLRequest");

        let mut bytes = BytesMut::with_capacity(12);
        bytes.put_i32(8);
        bytes.put_i32(SSL_REQUEST);
        server_stream.write_all(&bytes).await?;

        let ssl_response = server_stream.read_u8().await?;

        match ssl_response {
            // Server supports TLS
            b'S' => {
                let mut root_cert_store = rustls::RootCertStore::empty();
                root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                // for cert in rustls_native_certs::load_native_certs().expect("could not load platform certs")
                // {
                //     root_cert_store.add(cert).unwrap();
                // }

                let mut config = rustls::ClientConfig::builder()
                    .with_root_certificates(root_cert_store)
                    .with_no_client_auth();

                let mut dangerous = config.dangerous();
                dangerous.set_certificate_verifier(Arc::new(NoCertificateVerification {}));

                info!("Connecting to database over TLS");

                let connector = TlsConnector::from(Arc::new(config));

                let domain = encrypt.config.server.server_name()?.to_owned();
                let tls_stream = connector.connect(domain, server_stream).await?;

                server_stream = tls_stream.into_inner().0;
            }

            // Server does not support TLS
            b'N' => {
                error!("Database cannot connect over TLS");
                return Err(ProtocolError::UnexpectedSSLResponse.into());
            }

            // Something else?
            code => {
                error!("Unexpected startup message: {}", code as char);
                return Err(ProtocolError::UnexpectedStartupMessage.into());
            }
        }
    };

    info!(
        database = encrypt.config.connect.to_socket_address(),
        tls = encrypt.config.server.use_tls,
        "Database connected"
    );

    let startup_message = pg::read_startup_message(client_stream).await?;
    info!("startup_message 1 {:?}", startup_message.code);

    match &startup_message.code {
        pg::StartupCode::SSLRequest => {
            // SSLRequest is always the first message sent by the client
            // Respond to the SSLRequest
            // N for no, S for yeS or tlS
            // The SSLResponse MUST come before the TLS handshake
            let response = match encrypt.config.tls {
                Some(_) => b'S',
                None => b'N',
            };

            debug!("SSLResponse: {}", response as char);

            // Write the SSLResponse to the client
            client_stream.write_all(&[response]).await?;

            // The TLS handshake MUST come after the SSLResponse
            if let Some(tls) = &encrypt.config.tls {
                let server_config = tls.server_config()?;
                let acceptor = TlsAcceptor::from(Arc::new(server_config));

                let mut tls_stream = acceptor.accept(&mut *client_stream).await?;

                debug!("Client TLS negotiation complete");

                // If startup message an SSLRequest, the next message MUST be the ProtocolVersionNumber
                let startup_message = pg::read_startup_message(&mut tls_stream).await?;
                info!("startup_message 2 {:?}", startup_message.code);

                match &startup_message.code {
                    pg::StartupCode::ProtocolVersionNumber => {
                        debug!("ProtocolVersionNumber");
                        server_stream.write_all(&startup_message.bytes).await?;
                        debug!("After ProtocolVersionNumber send");
                    }
                    _ => {
                        error!("Unexpected message after TLS negotiation");
                        return Err(ProtocolError::UnexpectedStartupMessage.into());
                    }
                }
                client_stream = tls_stream.into_inner().0;
            } else {
                // If startup message an SSLRequest, the next message MUST be the ProtocolVersionNumber
                let startup_message = pg::read_startup_message(client_stream).await?;

                match &startup_message.code {
                    pg::StartupCode::ProtocolVersionNumber => {
                        debug!("ProtocolVersionNumber");
                        server_stream.write_all(&startup_message.bytes).await?;
                    }
                    _ => {
                        error!("Unexpected message after TLS negotiation");
                        return Err(ProtocolError::UnexpectedStartupMessage.into());
                    }
                }
            }
        }
        pg::StartupCode::ProtocolVersionNumber => {
            debug!("ProtocolVersionNumber");
            server_stream.write_all(&startup_message.bytes).await?;
        }
        pg::StartupCode::CancelRequest => {
            debug!("CancelRequest");
            // propagate the cancel request to the server and end the connection
            server_stream.write_all(&startup_message.bytes).await?;
            return Err(Error::CancelRequest);
        }
    }

    let (mut client_reader, mut client_writer) = split(client_stream);
    let (mut server_reader, mut server_writer) = split(server_stream);

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
