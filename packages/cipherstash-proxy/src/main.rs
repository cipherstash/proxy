use cipherstash_proxy::config::TandemConfig;
use cipherstash_proxy::connect::{self, AsyncStream};
use cipherstash_proxy::encrypt::Encrypt;
use cipherstash_proxy::error::Error;
use cipherstash_proxy::prometheus::CLIENTS_ACTIVE_CONNECTIONS;
use cipherstash_proxy::{cli, log, postgresql as pg, prometheus, tls, Args};
use clap::Parser;
use metrics::gauge;
use tokio::net::TcpListener;
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let config = match TandemConfig::load(&args) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Configuration Error: {}", err);
            std::process::exit(exitcode::CONFIG);
        }
    };

    log::init(config.log.clone());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.server.worker_threads)
        .thread_stack_size(config.thread_stack_size())
        .enable_all()
        .build()?;

    // Run any CLI commands
    let args_clone = args.clone();
    let config_clone = config.clone();
    runtime.block_on(async move {
        match cli::run(args_clone, config_clone).await {
            Ok(exit) => {
                if exit {
                    std::process::exit(exitcode::OK);
                }
            }

            Err(err) => {
                error!(msg = "Error running command", error = err.to_string());
                std::process::exit(exitcode::USAGE);
            }
        }
    });

    runtime.block_on(async move {
        let shutdown_timeout = &config.server.shutdown_timeout();

        let mut encrypt = init(config).await;

        let mut listener = connect::bind_with_retry(&encrypt.config.server).await;
        let tracker = TaskTracker::new();

        let mut client_id = 0;

        if encrypt.config.prometheus_enabled() {
            let host = encrypt.config.server.host.to_owned();
            match prometheus::start(host, encrypt.config.prometheus.port) {
                Ok(_) => {}
                Err(err) => {
                    error!(
                        msg = "Could not start CipherStash proxy",
                        error = err.to_string()
                    );
                    std::process::exit(exitcode::CONFIG);
                }
            }
        }

    loop {
        tokio::select! {
            _ = sigint() => {
                info!(msg = "Received SIGINT");
                break;
            },
            _ = sighup() => {
                info!(msg = "Received SIGHUP. Reloading configuration");
                (listener, encrypt) = reload_config(listener, &args, encrypt).await;
                info!(msg = "Reloaded configuration");
            },
            _ = sigterm() => {
                info!(msg = "Received SIGTERM");
                break;
            },
            Ok(client_stream) = AsyncStream::accept(&listener) => {

                    let encrypt = encrypt.clone();

                    client_id += 1;

                    tracker.spawn(async move {
                        let encrypt = encrypt.clone();

                        gauge!(CLIENTS_ACTIVE_CONNECTIONS).increment(1);

                        match pg::handler(client_stream, encrypt, client_id).await {
                            Ok(_) => (),
                            Err(err) => {

                                gauge!(CLIENTS_ACTIVE_CONNECTIONS).decrement(1);

                                match err {
                                    Error::ConnectionClosed => {
                                        info!(msg = "Database connection closed by client");
                                    }
                                    Error::CancelRequest => {
                                        info!(msg = "Database connection closed after cancel request");
                                    }
                                    Error::ConnectionTimeout(_) => {
                                        warn!(msg = "Database connection timeout");
                                    }
                                    _ => {
                                        error!(msg = "Database connection error", error = err.to_string());
                                    }
                                }
                            },
                        }
                    });
                },
            }
        }

        info!(msg = "Shutting down CipherStash Proxy");

        // Close the listener
        drop(listener);

        tracker.close();

        info!(msg = "Waiting for clients");

        if (tokio::time::timeout(*shutdown_timeout, tracker.wait()).await).is_err() {
            warn!(msg = "Terminated client connections", count = tracker.len());
        }
    });
    Ok(())
}

///
/// Validate various configuration options and
/// Init the Encrypt service
///
async fn init(mut config: TandemConfig) -> Encrypt {
    if config.encrypt.dataset_id.is_none() {
        info!(msg = "Encrypt using default dataset");
    }

    match config.server.server_name() {
        Ok(_) => {}
        Err(err) => {
            error!(
                msg = "Could not start CipherStash proxy",
                error = err.to_string()
            );
            std::process::exit(exitcode::CONFIG);
        }
    }

    if !config.database.with_tls_verification {
        warn!(
            msg = "Bypassing Transport Layer Security (TLS) verification for database connections"
        );
    }

    if config.mapping_disabled() {
        warn!(msg = "Mapping is not enabled");
    }

    match config.tls {
        Some(ref mut tls) => {
            if !tls.cert_exists() {
                error!(
                    msg = tls.certificate_err_msg(),
                    certificate = ?tls.certificate().lines().next().unwrap_or("") // show first line of PEM, or path (_should_ be 1 line)
                );
                std::process::exit(exitcode::CONFIG);
            }

            if !tls.private_key_exists() {
                error!(
                    msg = tls.private_key_err_msg(),
                    private_key = ?tls.private_key().lines().next().unwrap_or("") // show first line of PEM, or path (_should_ be 1 line)
                );
                std::process::exit(exitcode::CONFIG);
            };

            match tls::configure_server(tls) {
                Ok(_) => {
                    info!(msg = "Server Transport Layer Security (TLS) configuration validated");
                }
                Err(err) => {
                    error!(
                        msg = "Server Transport Layer Security (TLS) configuration error",
                        error = err.to_string()
                    );
                    std::process::exit(exitcode::CONFIG);
                }
            }
        }
        None => {
            warn!(msg = "Transport Layer Security (TLS) is not configured");
            warn!(msg = "Listening on an unsafe connection");
        }
    }

    match Encrypt::init(config).await {
        Ok(encrypt) => {
            info!(msg = "Connected to CipherStash Encrypt");
            info!(
                msg = "Connected to Database",
                database = encrypt.config.database.name,
                host = encrypt.config.database.host,
                port = encrypt.config.database.port,
                username = encrypt.config.database.username,
            );
            encrypt
        }
        Err(err) => {
            error!(
                msg = "Could not start CipherStash proxy",
                error = err.to_string()
            );
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
    args: &Args,
    encrypt: Encrypt,
) -> (TcpListener, Encrypt) {
    let new_config = match TandemConfig::load(args) {
        Ok(config) => config,
        Err(err) => {
            warn!(
                msg = "Configuration could not be reloaded: {}",
                error = err.to_string()
            );
            return (listener, encrypt);
        }
    };

    let new_encrypt = init(new_config).await;

    // Explicit drop needed here to free the network resources before binding if using the same address & port
    std::mem::drop(listener);

    (
        connect::bind_with_retry(&new_encrypt.config.server).await,
        new_encrypt,
    )
}
