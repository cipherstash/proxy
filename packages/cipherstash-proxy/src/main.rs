use cipherstash_proxy::config::TandemConfig;
use cipherstash_proxy::connect::{self, AsyncStream};
use cipherstash_proxy::encrypt::Encrypt;
use cipherstash_proxy::error::Error;
use cipherstash_proxy::{log, postgresql as pg, tls};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, error, info, warn};

// TODO: Accept command line arguments for config file path
#[tokio::main]
async fn main() {
    let config_file = "cipherstash-proxy.toml";

    log::init();

    let config = match TandemConfig::load(config_file) {
        Ok(config) => config,
        Err(err) => {
            error!("Configuration Error: {}", err);
            std::process::exit(exitcode::CONFIG);
        }
    };

    let encrypt = init(config).await;

    let listener = connect::bind_with_retry(&encrypt.config.server).await;

    loop {
        tokio::select! {
            _ = sigint() => {
                info!("Received SIGINT");
                break;
            },
            _ = sighup() => {
                info!("Received SIGHUP");
                break;
            },
            _ = sigterm() => {
                info!("Received SIGTERM");
                break;
            },
            Ok(client_stream) = AsyncStream::accept(&listener) => {

                let encrypt = encrypt.clone();

                tokio::spawn(async move {
                    let encrypt = encrypt.clone();

                    match pg::handler(client_stream, encrypt).await {
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
