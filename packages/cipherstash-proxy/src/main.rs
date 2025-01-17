use cipherstash_proxy::config::TandemConfig;
use cipherstash_proxy::connect::{self, AsyncStream};
use cipherstash_proxy::encrypt::Encrypt;
use cipherstash_proxy::error::Error;
use cipherstash_proxy::{log, postgresql as pg, tls};
use tokio::net::TcpListener;
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info, warn};

// TODO: Accept command line arguments for config file path
#[tokio::main]
async fn main() {
    let config_file = "cipherstash-proxy.toml";

    let config = match TandemConfig::load(config_file) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Configuration Error: {}", err);
            std::process::exit(exitcode::CONFIG);
        }
    };

    log::init(config.log.clone());

    let shutdown_timeout = &config.server.shutdown_timeout();

    let mut encrypt = init(config).await;

    let mut listener = connect::bind_with_retry(&encrypt.config.server).await;
    let tracker = TaskTracker::new();

    let mut client_id = 0;

    loop {
        tokio::select! {
            _ = sigint() => {
                info!("Received SIGINT");
                break;
            },
            _ = sighup() => {
                info!("Received SIGHUP. Reloading configuration");
                (listener, encrypt) = reload_config(listener, config_file, encrypt).await;
                info!("Finished reloading configuration");
            },
            _ = sigterm() => {
                info!("Received SIGTERM");
                break;
            },
            Ok(client_stream) = AsyncStream::accept(&listener) => {

                let encrypt = encrypt.clone();

                client_id += 1;

                tracker.spawn(async move {
                    let encrypt = encrypt.clone();

                    match pg::handler(client_stream, encrypt, client_id).await {
                        Ok(_) => (),
                        Err(e) => match e {
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
                        },
                    }
                });
            },
        }
    }

    info!("Shutting down CipherStash Proxy");

    // Close the listener
    drop(listener);

    tracker.close();

    info!("Waiting for clients");

    if let Err(_) = tokio::time::timeout(*shutdown_timeout, tracker.wait()).await {
        warn!("Terminated {} client connections", tracker.len());
    }
}

///
/// Validate various configuration options and
/// Init the Encrypt service
///
async fn init(mut config: TandemConfig) -> Encrypt {
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

    if !config.database.with_tls_verification {
        warn!("Bypassing Transport Layer Security (TLS) verification for database connections");
    }

    if config.disable_mapping() {
        warn!("Mapping is not enabled");
    }

    match config.tls {
        Some(ref mut tls) => {
            if !tls.cert_exists() {
                error!(
                    "Transport Layer Security (TLS) Certificate not found: {}",
                    tls.certificate
                );
                std::process::exit(exitcode::CONFIG);
            }

            if !tls.private_key_exists() {
                error!(
                    "Transport Layer Security (TLS) Private key not found: {}",
                    tls.private_key
                );
                std::process::exit(exitcode::CONFIG);
            };

            match tls::configure_server(tls) {
                Ok(_) => {
                    info!("Server Transport Layer Security (TLS) configuration validated");
                }
                Err(err) => {
                    error!("Server Transport Layer Security (TLS) configuration error");
                    error!("{}", err);
                    std::process::exit(exitcode::CONFIG);
                }
            }
        }
        None => {
            warn!("Transport Layer Security (TLS) is not configured");
            warn!("Listening on an unsafe connection");
        }
    }

    match Encrypt::init(config).await {
        Ok(encrypt) => {
            info!("Connected to CipherStash Encrypt");
            info!("Connected to database: {}", encrypt.config.database);
            encrypt
        }
        Err(err) => {
            error!("Could not start CipherStash proxy");
            debug!("{}", err);
            std::process::exit(exitcode::UNAVAILABLE);
        }
    }
}

async fn sigint() -> std::io::Result<()> {
    signal(SignalKind::interrupt())?.recv().await;
    Ok(())
}

async fn sigterm() -> std::io::Result<()> {
    signal(SignalKind::terminate())?.recv().await;
    Ok(())
}

async fn sighup() -> std::io::Result<()> {
    signal(SignalKind::hangup())?.recv().await;
    Ok(())
}

async fn reload_config(
    listener: TcpListener,
    config_file: &str,
    encrypt: Encrypt,
) -> (TcpListener, Encrypt) {
    let new_config = match TandemConfig::load(config_file) {
        Ok(config) => config,
        Err(err) => {
            warn!("Configuration could not be reloaded: {}", err);
            return (listener, encrypt);
        }
    };

    let new_encrypt = init(new_config).await;

    // TODO: if it is not too hard to implement PartialEq for Encrypt, it would be great to check for changes
    // and skip reloading if nothing has changed

    // Explicit drop needed here to free the network resources before binding if using the same address & port
    std::mem::drop(listener);

    (
        connect::bind_with_retry(&new_encrypt.config.server).await,
        new_encrypt,
    )
}
