mod async_stream;
mod channel_writer;

pub use async_stream::AsyncStream;
pub use channel_writer::{ChannelWriter, Sender};

use crate::{config::ServerConfig, error::Error, log::DEVELOPMENT, tls, DatabaseConfig};
use socket2::TcpKeepalive;
use std::time::Duration;
use tokio::{
    net::{TcpListener, TcpStream},
    time::{self},
};
use tokio_postgres::Client;
use tracing::{debug, error, info, warn};

const TCP_USER_TIMEOUT: Duration = Duration::from_secs(10);
const TCP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const TCP_KEEPALIVE_TIME: Duration = Duration::from_secs(5);
const TCP_KEEPALIVE_RETRIES: u32 = 5;

const MAX_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_COUNT: u32 = 3;

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

pub async fn bind_with_retry(server: &ServerConfig, allow_random_fallback: bool) -> TcpListener {
    let address = server.to_socket_address();
    let mut retry_count = 0;

    loop {
        match TcpListener::bind(&address).await {
            Ok(listener) => {
                report_listening(&listener);
                return listener;
            }
            // The configured port is in use, but it's only the default (not
            // explicitly set), so fall back to an OS-assigned port and report it
            // rather than failing.
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse && allow_random_fallback => {
                match TcpListener::bind(format!("{}:0", server.host)).await {
                    Ok(listener) => {
                        println!(
                            "Port {} is already in use; listening on an OS-assigned port instead.",
                            server.port
                        );
                        report_listening(&listener);
                        return listener;
                    }
                    Err(err) => {
                        error!(msg = "Error binding connection", error = err.to_string());
                        std::process::exit(exitcode::CONFIG);
                    }
                }
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

/// Report the address the proxy is listening on. Uses `println!` so the
/// listening address (including an OS-assigned fallback port) is always visible,
/// even when logging is quiet.
fn report_listening(listener: &TcpListener) {
    match listener.local_addr() {
        Ok(addr) => {
            println!("CipherStash Proxy listening on {addr}");
            info!(msg = "Server waiting for connections", address = %addr);
        }
        Err(_) => {
            info!(msg = "Server waiting for connections");
        }
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

    #[cfg(target_os = "linux")]
    match sock_ref.set_tcp_user_timeout(Some(TCP_USER_TIMEOUT)) {
        Ok(_) => (),
        Err(err) => {
            warn!(
                msg = "Error configuring tcp_user_timeout for connection",
                error = err.to_string()
            );
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_falls_back_to_random_port_when_default_in_use() {
        // Occupy a port, then ask bind_with_retry to bind the same one.
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();

        let server = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: occupied_port,
            ..ServerConfig::default()
        };

        // With fallback allowed, it binds a different OS-assigned port.
        let listener = bind_with_retry(&server, true).await;
        let bound_port = listener.local_addr().unwrap().port();

        assert_ne!(bound_port, occupied_port);
        assert_ne!(bound_port, 0);
    }
}
