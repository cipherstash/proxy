use crate::error::Error;
use socket2::TcpKeepalive;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{split, AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::TlsStream;

use super::configure;

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

    pub async fn connect(addr: &str) -> Result<AsyncStream, Error> {
        let mut stream = TcpStream::connect(addr).await?;
        configure(&mut stream);
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
