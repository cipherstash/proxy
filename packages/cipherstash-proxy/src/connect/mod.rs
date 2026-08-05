mod channel_writer;

pub use channel_writer::{ChannelWriter, Sender};

use crate::{config::ServerConfig, error::Error, tls, DatabaseConfig};
use pg_proto::net::{ConnectRetry, NetworkStream, TcpSettings};
use std::time::Duration;
use tokio::{
    net::{TcpListener, TcpStream},
    time::{self},
};
use tokio_postgres::Client;
use tracing::{debug, error, info, warn};

const MAX_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_COUNT: u32 = 3;
const TCP_USER_TIMEOUT: Duration = Duration::from_secs(10);
const TCP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const TCP_KEEPALIVE_TIME: Duration = Duration::from_secs(5);
const TCP_KEEPALIVE_RETRIES: u32 = 5;

fn configure_tcp(stream: &TcpStream) {
    let settings = TcpSettings {
        no_delay: true,
        user_timeout: Some(TCP_USER_TIMEOUT),
        keepalive_time: Some(TCP_KEEPALIVE_TIME),
        keepalive_interval: Some(TCP_KEEPALIVE_INTERVAL),
        keepalive_retries: Some(TCP_KEEPALIVE_RETRIES),
    };
    for error in pg_proto::net::configure_tcp(stream, settings) {
        warn!(msg = "Error configuring connection", error = %error);
    }
}

pub async fn accept(listener: &TcpListener) -> Result<NetworkStream<TcpStream>, Error> {
    let (stream, _) = listener.accept().await?;
    configure_tcp(&stream);
    Ok(NetworkStream::plain(stream))
}

pub async fn connect(address: &str) -> Result<NetworkStream<TcpStream>, Error> {
    debug!(msg = "Connecting to database");
    let retry = ConnectRetry {
        max_retries: MAX_RETRY_COUNT,
        initial_delay: Duration::from_millis(100),
        max_delay: MAX_RETRY_DELAY,
    };
    let stream = pg_proto::net::connect_with_retry(address, retry)
        .await
        .map_err(|error| {
            error!(msg = "Could not connect to database", error = %error);
            Error::DatabaseConnection
        })?;
    configure_tcp(&stream);
    Ok(NetworkStream::plain(stream))
}

pub async fn database(config: &DatabaseConfig) -> Result<Client, Error> {
    let connection_config = config.to_connection_config();

    let tls_config = tls::configure_client(config);
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);

    let (client, connection) = match connection_config.connect(tls).await {
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
