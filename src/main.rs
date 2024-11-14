mod config;
mod encrypt;
mod eql;
mod error;
mod postgresql;

use config::TandemConfig;
use encrypt::Encrypt;
use error::Error;
use std::mem;
use std::sync::{Arc, Once};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

static INIT: Once = Once::new();

const SIZE_U8: usize = mem::size_of::<u8>();
const SIZE_I16: usize = mem::size_of::<i16>();
const SIZE_I32: usize = mem::size_of::<i32>();

const URL: &str = "127.0.0.1:6432";

// const URL: &str = "127.0.0.1:6433";
// const DOWNSTREAM: &str = "127.0.0.1:1433";

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

    startup(&config).await;

    info!("config: {:?}", config);

    let encrypt = match Encrypt::init(config).await {
        Ok(encrypt) => encrypt,
        Err(err) => {
            error!("Failed to initialise : {}", err);
            std::process::exit(exitcode::UNAVAILABLE);
        }
    };

    let listener = TcpListener::bind(URL).await.unwrap();
    info!(url = URL, "Server listening");

    loop {
        let (mut socket, _) = listener.accept().await.unwrap();
        let encrypt = encrypt.clone();
        tokio::spawn(async move {
            loop {
                match handle(encrypt.clone(), &mut socket).await {
                    Ok(_) => (),
                    Err(e) => {
                        match e {
                            Error::ConnectionClosed => {
                                info!("Connection closed by client");
                            }
                            Error::ConnectionTimeout(_) => {
                                warn!("Connection timeout");
                            }
                            _ => {
                                error!("Error {:?}", e);
                            }
                        }
                        break;
                    }
                }
            }
        });
    }
}

async fn startup(config: &TandemConfig) {
    if config.encrypt.dataset_id.is_none() {
        info!("Encrypt: using default dataset");
    }
}

async fn handle(encrypt: Encrypt, client: &mut TcpStream) -> Result<(), Error> {
    let mut server = TcpStream::connect(&encrypt.config.connect.database).await?;

    info!(database = encrypt.config.connect.database, "Connected");

    let (client_reader, mut client_writer) = client.split();
    let (server_reader, mut server_writer) = server.split();

    let client_to_server = async {
        let mut fe = postgresql::Frontend::new(client_reader, server_writer);
        loop {
            let bytes = fe.read().await?;

            debug!("[client_to_server]");
            debug!("bytes: {bytes:?}");

            fe.write(bytes).await?;
            debug!("write complete");
        }

        // Unreachable, but helps the compiler understand the return type
        // TODO: extract into a function
        Ok::<(), Error>(())
    };

    let server_to_client = async {
        let mut be = postgresql::Backend::new(server_reader);

        loop {
            let bytes = be.read().await?;

            debug!("[client_to_server]");
            debug!("bytes: {bytes:?}");

            client_writer.write_all(&bytes).await?;
            debug!("write complete");
        }

        Ok::<(), Error>(())
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}

fn trace() {
    INIT.call_once(|| {
        use tracing_subscriber::FmtSubscriber;

        let subscriber = FmtSubscriber::builder()
            .with_max_level(tracing::Level::DEBUG) // Set the maximum level of tracing events that should be logged.
            .with_line_number(true)
            .with_target(true)
            .finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    });
}
