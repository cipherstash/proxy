mod async_stream;

use crate::{config::ServerConfig, error::Error, DatabaseConfig};
use socket2::TcpKeepalive;
use std::time::Duration;

use tokio::{
    net::{TcpListener, TcpStream},
    time::{self},
};
use tokio_postgres::{Client, NoTls};
use tracing::{debug, error, info, warn};

pub use async_stream::AsyncStream;

const TCP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const TCP_KEEPALIVE_TIME: Duration = Duration::from_secs(5);
const TCP_KEEPALIVE_RETRIES: u32 = 5;

const MAX_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_COUNT: u32 = 3;

pub async fn database(config: &DatabaseConfig) -> Result<Client, tokio_postgres::Error> {
    let connection_string = config.to_connection_string();

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    Ok(client)
}

pub async fn bind_with_retry(server: &ServerConfig) -> TcpListener {
    let address = &server.to_socket_address();
    let mut retry_count = 0;

    loop {
        info!("Attempting to bind server connection {address}");
        match TcpListener::bind(address).await {
            Ok(listener) => {
                info!(address = address, "Server connected");
                return listener;
            }
            Err(err) => {
                if retry_count > MAX_RETRY_COUNT {
                    error!("Error binding connection after {MAX_RETRY_COUNT} retries");
                    error!("{err}");
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
        info!("Connecting to database");
        match TcpStream::connect(&addr).await {
            Ok(stream) => {
                return Ok(stream);
            }
            Err(err) => {
                if retry_count > MAX_RETRY_COUNT {
                    error!("{err}");
                    return Err(Error::DatabaseConnection {
                        retries: MAX_RETRY_COUNT,
                    });
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
/// TODO stay on target
/// TODO should this be configurable?
/// Keepalive is not as important without connection pooling timeouts to deal with
///
pub fn configure(stream: &TcpStream) {
    let sock_ref = socket2::SockRef::from(&stream);

    stream.set_nodelay(true).unwrap_or_else(|err| {
        warn!("Error configuring nodelay for connection");
        debug!("{err}");
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
                    warn!("Error configuring keepalive for connection");
                    debug!("{err}");
                }
            }
        }
        Err(err) => {
            warn!("Error configuring connection");
            debug!("{err}");
        }
    }
}
