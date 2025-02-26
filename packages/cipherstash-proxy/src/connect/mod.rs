mod async_stream;

use crate::{config::ServerConfig, error::Error, log::DEVELOPMENT, tls, DatabaseConfig};
use socket2::TcpKeepalive;
use std::time::Duration;
use tokio::{
    net::{TcpListener, TcpStream},
    time::{self},
};
use tokio_postgres::Client;
use tracing::{debug, error, info, warn};

pub use async_stream::{AsyncStream, ChannelWriter};

const TCP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const TCP_KEEPALIVE_TIME: Duration = Duration::from_secs(5);
const TCP_KEEPALIVE_RETRIES: u32 = 5;

const MAX_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_COUNT: u32 = 3;

pub async fn database(config: &DatabaseConfig) -> Result<Client, Error> {
    let connection_string = config.to_connection_string();

    let tls_config = tls::configure_client(config);
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);

    let (client, connection) = match tokio_postgres::connect(&connection_string, tls).await {
        Ok((client, connection)) => (client, connection),
        Err(e) => {
            error!(
                msg = "Could not connect to database",
                database = config.name,
                host = config.host,
                port = config.port,
                username = config.username,
            );
            error!(msg = "Confirm that the database configuration is correct");
            return Err(Error::Config(e.into()));
        }
    };

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            error!(msg = "Connection error", error = err.to_string());
        }
    });
    Ok(client)
}

pub async fn bind_with_retry(server: &ServerConfig) -> TcpListener {
    let address = &server.to_socket_address();
    let mut retry_count = 0;

    loop {
        match TcpListener::bind(address).await {
            Ok(listener) => {
                info!(msg = "Server waiting for connections", address);
                return listener;
            }
            Err(err) => {
                if retry_count > MAX_RETRY_COUNT {
                    error!(
                        msg = "Error binding connection",
                        retries = MAX_RETRY_COUNT,
                        error = err.to_string()
                    );
                    std::process::exit(exitcode::CONFIG);
                }
            }
        };
        let sleep_duration_ms =
            (100 * 2_u64.pow(retry_count)).min(MAX_RETRY_DELAY.as_millis() as _);
        time::sleep(Duration::from_millis(sleep_duration_ms)).await;

        retry_count += 1;
    }
}

pub async fn connect_with_retry(addr: &str) -> Result<TcpStream, Error> {
    let mut retry_count = 0;

    loop {
        debug!(target: DEVELOPMENT, msg = "Connecting to database");
        match TcpStream::connect(&addr).await {
            Ok(stream) => {
                return Ok(stream);
            }
            Err(err) => {
                if retry_count > MAX_RETRY_COUNT {
                    error!(msg = "Could not connect to database", retries = ?retry_count, error = err.to_string());
                    return Err(Error::DatabaseConnection);
                }
            }
        };
        let sleep_duration_ms =
            (100 * 2_u64.pow(retry_count)).min(MAX_RETRY_DELAY.as_millis() as _);
        time::sleep(Duration::from_millis(sleep_duration_ms)).await;

        retry_count += 1;
    }
}

///
/// Configure the tcp socket
///     set_nodelay
///     set_keepalive
///
/// Keepalive is not as important without connection pooling timeouts to deal with
///
pub fn configure(stream: &TcpStream) {
    let sock_ref = socket2::SockRef::from(&stream);

    stream.set_nodelay(true).unwrap_or_else(|err| {
        warn!(
            msg = "Error configuring nodelay for connection",
            error = err.to_string()
        );
    });

    match sock_ref.set_keepalive(true) {
        Ok(_) => {
            let params = &TcpKeepalive::new()
                .with_interval(TCP_KEEPALIVE_INTERVAL)
                .with_retries(TCP_KEEPALIVE_RETRIES)
                .with_time(TCP_KEEPALIVE_TIME);

            match sock_ref.set_tcp_keepalive(params) {
                Ok(_) => (),
                Err(err) => {
                    warn!(
                        msg = "Error configuring keepalive for connection",
                        error = err.to_string()
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                msg = "Error configuring connection",
                error = err.to_string()
            );
        }
    }
}
