use socket2::TcpKeepalive;
use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{split, AsyncRead, AsyncWrite, ReadBuf, ReadHalf, WriteHalf},
    net::{TcpListener, TcpStream},
    time,
};
use tokio_rustls::TlsStream;
use tracing::{debug, error, info, warn};

use crate::{config::ServerConfig, error::Error};

const INTERVAL: Duration = Duration::from_secs(5);
const TIME: Duration = Duration::from_secs(5);
const RETRIES: u32 = 5;

const MAX_BACKOFF: Duration = Duration::from_secs(2);
const MAX_RETRY_COUNT: u32 = 3;

#[derive(Debug)]
pub enum AsyncStream {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl AsyncStream {
    pub async fn accept(listener: &TcpListener) -> Result<AsyncStream, Error> {
        let (stream, _) = listener.accept().await?;
        configure(&stream);
        Ok(AsyncStream::Tcp(stream))
    }

    pub async fn split(
        self,
    ) -> (
        tokio::io::ReadHalf<AsyncStream>,
        tokio::io::WriteHalf<AsyncStream>,
    ) {
        split(self)
    }
}

impl AsyncRead for AsyncStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match *self {
            AsyncStream::Tcp(ref mut stream) => Pin::new(stream).poll_read(cx, buf),
            AsyncStream::Tls(ref mut stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for AsyncStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match *self {
            AsyncStream::Tcp(ref mut stream) => Pin::new(stream).poll_write(cx, buf),
            AsyncStream::Tls(ref mut stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match *self {
            AsyncStream::Tcp(ref mut stream) => Pin::new(stream).poll_flush(cx),
            AsyncStream::Tls(ref mut stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match *self {
            AsyncStream::Tcp(ref mut stream) => Pin::new(stream).poll_shutdown(cx),
            AsyncStream::Tls(ref mut stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

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

// pub fn configure_socket(stream: &TcpStream) {
//     let sock_ref = SockRef::from(stream);
//     let conf = get_config();

//     #[cfg(target_os = "linux")]
//     match sock_ref.set_tcp_user_timeout(Some(Duration::from_millis(conf.general.tcp_user_timeout)))
//     {
//         Ok(_) => (),
//         Err(err) => error!("Could not configure tcp_user_timeout for socket: {}", err),
//     }

//     sock_ref.set_nodelay(true).unwrap_or_else(|err| {
//         warn!("Could not configure nodelay for socket: {}", err);
//     });

//     match sock_ref.set_keepalive(true) {
//         Ok(_) => {
//             match sock_ref.set_tcp_keepalive(
//                 &TcpKeepalive::new()
//                     .with_interval(Duration::from_secs(conf.general.tcp_keepalives_interval))
//                     .with_retries(conf.general.tcp_keepalives_count)
//                     .with_time(Duration::from_secs(conf.general.tcp_keepalives_idle)),
//             ) {
//                 Ok(_) => (),
//                 Err(err) => error!("Could not configure tcp_keepalive for socket: {}", err),
//             }
//         }
//         Err(err) => error!("Could not configure socket: {}", err),
//     }
// }
