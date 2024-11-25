use my_little_proxy::config::TandemConfig;
use my_little_proxy::connect::AsyncStream;
use my_little_proxy::encrypt::Encrypt;
use my_little_proxy::error::Error;
use my_little_proxy::{postgresql as pg, trace};
use tokio::net::{tcp, TcpListener, TcpStream};
use tracing::{error, info, warn};

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
    let encrypt = init(config).await;
    // let listener = tcp::bind_with_retry(&encrypt.config.server).await;
    let listener = TcpListener::bind(&encrypt.config.server.to_socket_address())
        .await
        .unwrap();

    loop {
        // let (stream, _) = listener.accept().await.unwrap();
        let client_stream = AsyncStream::accept(&listener).await.unwrap();

        let encrypt = encrypt.clone();

        tokio::spawn(async move {
            let encrypt = encrypt.clone();

            match pg::handle(client_stream, encrypt).await {
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
async fn init(config: TandemConfig) -> Encrypt {
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
