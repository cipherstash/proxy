use cipherstash_proxy::config::{LogConfig, LogLevel, TandemConfig};
use cipherstash_proxy::connect::{self, AsyncStream};
use cipherstash_proxy::error::{ConfigError, Error};
use cipherstash_proxy::prometheus::CLIENTS_ACTIVE_CONNECTIONS;
use cipherstash_proxy::proxy::Proxy;
use cipherstash_proxy::{cli, log, postgresql as pg, prometheus, tls, Args};
use clap::Parser;
use metrics::gauge;
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};

const EQL_VERSION_AT_BUILD_TIME: Option<&'static str> = option_env!("EQL_VERSION_AT_BUILD_TIME");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let config = match TandemConfig::load(&args) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(exitcode::CONFIG);
        }
    };

    // Quiet by default for CLI use: log errors only, unless --debug is passed,
    // a log level was set explicitly (--log-level / CS_LOG__LEVEL), or a config
    // file is present. This keeps `proxy` clean to run by hand while leaving
    // production (which sets a level or ships a config) unchanged.
    let log_explicitly_set = args.log_level != LogConfig::default_log_level()
        || std::env::var("CS_LOG__LEVEL").is_ok()
        || std::path::Path::new(&args.config_file_path).exists();
    let log_config = if args.debug {
        LogConfig::with_level(LogLevel::Debug)
    } else if log_explicitly_set {
        config.log.clone()
    } else {
        LogConfig::with_level(LogLevel::Error)
    };
    log::init(log_config);

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

        let mut proxy = init(config, args.tls).await;

        // If the listen port is just the default (not set via CS_SERVER__PORT or
        // a config file) and it's already in use, fall back to an OS-assigned
        // port rather than failing. An explicitly configured port is respected.
        let port_configured = std::env::var("CS_SERVER__PORT").is_ok()
            || std::path::Path::new(&args.config_file_path).exists();
        let listener = connect::bind_with_retry(&proxy.config.server, !port_configured).await;
        let tracker = TaskTracker::new();

        let mut client_id = 0;

        if proxy.config.prometheus_enabled() {
            let host = proxy.config.server.host.to_owned();
            match prometheus::start(host, proxy.config.prometheus.port) {
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
                info!(msg = "Received SIGHUP. Reloading application configuration");
                proxy = reload_application_config(&proxy.config, &args).await.unwrap_or(proxy);
            },
            _ = sigterm() => {
                info!(msg = "Received SIGTERM");
                break;
            },
            Ok(client_stream) = AsyncStream::accept(&listener) => {

                    client_id += 1;

                    let context = proxy.context(client_id);

                    tracker.spawn(async move {

                        gauge!(CLIENTS_ACTIVE_CONNECTIONS).increment(1);

                        match pg::handler(client_stream,context).await {
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
                                    Error::ConnectionTimeout{..} => {
                                        warn!(msg = "Database connection timeout", error = err.to_string());
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
/// Init the Proxy service
///
async fn init(mut config: TandemConfig, force_tls: bool) -> Proxy {
    if config.encrypt.default_keyset_id.is_none() {
        warn!(msg = "Default Keyset Id has not been configured");
        warn!(msg = "A Keyset Identifier must be set using the `SET CIPHERSTASH.KEYSET_ID` or `SET CIPHERSTASH.KEYSET_NAME` commands");
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
        warn!(msg = "Encrypted statement mapping is not enabled");
    }

    if config.mapping_errors_enabled() {
        info!(msg = "Encrypted statement mapping errors are enabled");
    }

    let _ = rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .inspect_err(|err| {
            error!(msg = "Could not initalise the CryptoProvider", ?err);
            std::process::exit(exitcode::CONFIG);
        });

    // Validate inbound TLS if configured. By default, a TLS misconfiguration is
    // non-fatal: we warn and fall back to a plaintext listener. Passing --tls
    // makes any TLS problem fatal (no silent downgrade).
    let tls_error: Option<String> = match &config.tls {
        Some(tls) => {
            if let Err(err) = tls.check_cert() {
                Some(err.to_string())
            } else if let Err(err) = tls.check_private_key() {
                Some(err.to_string())
            } else if let Err(err) = tls::configure_server(tls) {
                Some(err.to_string())
            } else {
                None
            }
        }
        None => None,
    };

    match (&config.tls, tls_error, force_tls) {
        // TLS configured and valid.
        (Some(_), None, _) => {
            info!(msg = "Server Transport Layer Security (TLS) configuration validated");
        }
        // TLS configured but invalid, and --tls forces it: fatal.
        (Some(_), Some(err), true) => {
            eprintln!("TLS is required (--tls) but the configuration is invalid: {err}");
            std::process::exit(exitcode::CONFIG);
        }
        // TLS configured but invalid, no force: fall back to plaintext.
        (Some(_), Some(err), false) => {
            eprintln!(
                "TLS configuration is invalid ({err}); falling back to a plaintext listener. \
                 Pass --tls to make this fatal, or --no-tls to silence this warning."
            );
            config.tls = None;
            config.server.require_tls = false;
        }
        // No TLS configured, but --tls forces it: fatal.
        (None, _, true) => {
            eprintln!(
                "TLS is required (--tls) but no inbound TLS certificate is configured \
                 (set CS_TLS__CERTIFICATE_PATH / CS_TLS__PRIVATE_KEY_PATH)."
            );
            std::process::exit(exitcode::CONFIG);
        }
        // No TLS configured, no force: plaintext listener.
        (None, _, false) => {
            warn!(msg = "Transport Layer Security (TLS) is not configured");
            warn!(msg = "Listening on an unsafe connection");
        }
    }

    match Proxy::init(config).await {
        Ok(proxy) => {
            info!(msg = "Connected to CipherStash Proxy");
            info!(
                msg = "Connected to Database",
                database = proxy.config.database.name,
                host = proxy.config.database.host,
                port = proxy.config.database.port,
                username = proxy.config.database.username,
                eql_version = proxy.eql_version,
            );
            if proxy.eql_version.as_deref() != EQL_VERSION_AT_BUILD_TIME {
                warn!(
                    msg = "installed version of EQL is different to the version that Proxy was built with",
                    eql_build_version = EQL_VERSION_AT_BUILD_TIME,
                    eql_installed_version = proxy.eql_version,
                );
            }
            proxy
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

fn has_network_config_changed(current: &TandemConfig, new: &TandemConfig) -> bool {
    current.server.host != new.server.host
        || current.server.port != new.server.port
        || current.server.require_tls != new.server.require_tls
        || current.server.worker_threads != new.server.worker_threads
        || current.tls != new.tls
}

async fn reload_application_config(config: &TandemConfig, args: &Args) -> Result<Proxy, Error> {
    let new_config = match TandemConfig::load(args) {
        Ok(config) => config,
        Err(err) => {
            warn!(
                msg = "Configuration could not be reloaded: {}",
                error = err.to_string()
            );
            return Err(err);
        }
    };

    // Check for network config changes that require restart
    if has_network_config_changed(config, &new_config) {
        let err = ConfigError::NetworkConfigurationChangeRequiresRestart;
        warn!(msg = err.to_string());

        return Err(err.into());
    }

    info!(msg = "Configuration reloaded");
    let proxy = init(new_config, args.tls).await;
    Ok(proxy)
}
