mod async_stream;

use crate::{config::ServerConfig, error::Error};
use socket2::TcpKeepalive;
use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{split, AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    time,
};
use tokio_rustls::TlsStream;
use tracing::{debug, error, info, warn};

pub use async_stream::AsyncStream;

const INTERVAL: Duration = Duration::from_secs(5);
const TIME: Duration = Duration::from_secs(5);
const RETRIES: u32 = 5;
const MAX_BACKOFF: Duration = Duration::from_secs(2);
const MAX_RETRY_COUNT: u32 = 3;

pub async fn bind_with_retry(server: &ServerConfig) -> TcpListener {
    let address = &server.to_socket_address();
    let mut retry_count = 0;

    loop {
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
                info!("Attempting to bind server connection {address}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        };
        let sleep_duration_ms = (100 * 2_u64.pow(retry_count)).min(MAX_BACKOFF.as_millis() as _);
        time::sleep(Duration::from_millis(sleep_duration_ms)).await;

        retry_count += 1;
    }
}

pub async fn connect_with_retry(addr: &str) -> TcpStream {
    let mut retry_count = 0;

    loop {
        match TcpStream::connect(&addr).await {
            Ok(stream) => {
                return stream;
            }
            Err(err) => {
                if retry_count > MAX_RETRY_COUNT {
                    error!("Error creating server connection after {MAX_RETRY_COUNT} retries");
                    error!("{err}");
                    std::process::exit(exitcode::CONFIG);
                }
                info!("Attempting to create server connection");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        };
        let sleep_duration_ms = (100 * 2_u64.pow(retry_count)).min(MAX_BACKOFF.as_millis() as _);
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
                .with_interval(INTERVAL)
                .with_retries(RETRIES)
                .with_time(TIME);

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
