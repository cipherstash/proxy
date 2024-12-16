use super::{configure, connect_with_retry};
use crate::error::Error;
use core::str;
use oid_registry::{
    Oid, OID_HASH_SHA1, OID_NIST_HASH_SHA256, OID_NIST_HASH_SHA384, OID_NIST_HASH_SHA512,
    OID_PKCS1_SHA1WITHRSA, OID_PKCS1_SHA256WITHRSA, OID_PKCS1_SHA384WITHRSA,
    OID_PKCS1_SHA512WITHRSA, OID_SIG_ECDSA_WITH_SHA256, OID_SIG_ECDSA_WITH_SHA384, OID_SIG_ED25519,
};
use postgres_protocol::authentication::sasl::ChannelBinding;
use ring::digest;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{split, AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::TlsStream;
use tracing::debug;
use x509_parser::prelude::{FromDer, X509Certificate};

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
        let stream = connect_with_retry(addr).await?;
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

    pub fn is_tls(&self) -> bool {
        matches!(self, AsyncStream::Tls(_))
    }

    pub fn is_tcp(&self) -> bool {
        !self.is_tls()
    }

    pub fn channel_binding(&self) -> ChannelBinding {
        match self {
            AsyncStream::Tcp(_) => {
                debug!("ChannelBinding is unsupported in TCP Connections");
                ChannelBinding::unsupported()
            }
            AsyncStream::Tls(stream) => {
                let (_, session) = stream.get_ref();
                let certs = session.peer_certificates();
                match certs {
                    Some(certs) if !certs.is_empty() => {
                        let cert_der = &certs[0];
                        X509Certificate::from_der(cert_der)
                            .ok()
                            .map(|(_, cert)| get_digest(&cert.signature_algorithm.algorithm))
                            .map_or_else(ChannelBinding::unsupported, |algorithm| {
                                let hash = digest::digest(algorithm, certs[0].as_ref());
                                ChannelBinding::tls_server_end_point(hash.as_ref().into())
                            })
                    }
                    _ => {
                        debug!("Missing certificates, ChannelBinding is unsupported");
                        ChannelBinding::unsupported()
                    }
                }
            }
        }
    }
}

///
/// Note: SHA1 is upgraded to SHA256 as per https://datatracker.ietf.org/doc/html/rfc5929#section-4.1
///
fn get_digest(oid: &Oid) -> &'static digest::Algorithm {
    match oid {
        oid if oid == &OID_HASH_SHA1 => &digest::SHA256,
        oid if oid == &OID_NIST_HASH_SHA256 => &digest::SHA256,
        oid if oid == &OID_PKCS1_SHA1WITHRSA => &digest::SHA256,
        oid if oid == &OID_PKCS1_SHA256WITHRSA => &digest::SHA256,
        oid if oid == &OID_SIG_ECDSA_WITH_SHA256 => &digest::SHA256,
        oid if oid == &OID_NIST_HASH_SHA384 => &digest::SHA384,
        oid if oid == &OID_PKCS1_SHA384WITHRSA => &digest::SHA384,
        oid if oid == &OID_SIG_ECDSA_WITH_SHA384 => &digest::SHA384,
        oid if oid == &OID_NIST_HASH_SHA512 => &digest::SHA512,
        oid if oid == &OID_PKCS1_SHA512WITHRSA => &digest::SHA512,
        oid if oid == &OID_SIG_ED25519 => &digest::SHA512,
        _ => panic!("Unsupported OID"),
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
