use crate::{
    config::ServerConfig,
    error::{Error, ErrorChain},
    tls, DatabaseConfig,
};
use socket2::{SockRef, TcpKeepalive};
use std::io::IsTerminal;
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
    if let Err(error) = stream.set_nodelay(true) {
        warn!(msg = "Error configuring TCP_NODELAY", error = %error);
    }

    let socket = SockRef::from(stream);
    #[cfg(target_os = "linux")]
    if let Err(error) = socket.set_tcp_user_timeout(Some(TCP_USER_TIMEOUT)) {
        warn!(msg = "Error configuring TCP_USER_TIMEOUT", error = %error);
    }

    if let Err(error) = socket.set_keepalive(true) {
        warn!(msg = "Error enabling TCP keepalive", error = %error);
        return;
    }

    let keepalive = TcpKeepalive::new()
        .with_interval(TCP_KEEPALIVE_INTERVAL)
        .with_time(TCP_KEEPALIVE_TIME)
        .with_retries(TCP_KEEPALIVE_RETRIES);
    if let Err(error) = socket.set_tcp_keepalive(&keepalive) {
        warn!(msg = "Error configuring TCP keepalive", error = %error);
    }
}

pub async fn accept(listener: &TcpListener) -> Result<TcpStream, Error> {
    let (stream, _) = listener.accept().await?;
    configure_tcp(&stream);
    Ok(stream)
}

pub async fn connect(address: &str) -> Result<TcpStream, Error> {
    debug!(msg = "Connecting to database");
    let mut delay = Duration::from_millis(100);
    for attempt in 0..=MAX_RETRY_COUNT {
        match TcpStream::connect(address).await {
            Ok(stream) => {
                configure_tcp(&stream);
                return Ok(stream);
            }
            Err(error) if attempt < MAX_RETRY_COUNT => {
                warn!(msg = "Database connection failed; retrying", %error, attempt);
                time::sleep(delay).await;
                delay = (delay * 2).min(MAX_RETRY_DELAY);
            }
            Err(error) => {
                error!(msg = "Could not connect to database", error = %error);
                return Err(Error::DatabaseConnection);
            }
        }
    }
    unreachable!()
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
            error!(msg = "Connection error", error = %ErrorChain(&err));
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
            // The port is still in use after retrying, but it's only the default
            // (not explicitly set), so fall back to an OS-assigned port and
            // report it rather than failing.
            Err(err)
                if err.kind() == std::io::ErrorKind::AddrInUse
                    && allow_random_fallback
                    && retry_count > MAX_RETRY_COUNT =>
            {
                match TcpListener::bind(format!("{}:0", server.host)).await {
                    Ok(listener) => {
                        if std::io::stderr().is_terminal() {
                            eprintln!(
                                "Port {} is already in use; listening on an OS-assigned port instead.",
                                server.port
                            );
                        }
                        warn!(
                            msg =
                                "Default port is already in use; listening on an OS-assigned port",
                            port = server.port
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

/// Report the address the proxy is listening on. In a terminal it is also
/// printed, so the listening address (including an OS-assigned fallback port)
/// is visible even when logging is quiet. Without a terminal the log line is
/// enough, and plain text would break a structured log stream.
fn report_listening(listener: &TcpListener) {
    match listener.local_addr() {
        Ok(addr) => {
            if std::io::stdout().is_terminal() {
                println!("CipherStash Proxy listening on {addr}");
            }
            info!(msg = "Server waiting for connections", address = %addr);
        }
        Err(_) => {
            info!(msg = "Server waiting for connections");
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

    #[tokio::test]
    /// The random-port fallback only applies once the retries are exhausted,
    /// so a port that frees up during the retries is still bound.
    async fn bind_retries_default_port_before_falling_back() {
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();

        let server = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: occupied_port,
            ..ServerConfig::default()
        };

        tokio::spawn(async move {
            time::sleep(Duration::from_millis(150)).await;
            drop(occupied);
        });

        let listener = bind_with_retry(&server, true).await;

        assert_eq!(listener.local_addr().unwrap().port(), occupied_port);
    }
}
