use crate::config::TlsConfig;
use crate::error::Error;
use rustls_pki_types::PrivateKeyDer;
use rustls_pki_types::{pem::PemObject, CertificateDer};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;
use tracing::debug;

use tokio_rustls::TlsAcceptor;

// pub async fn accept_tls(
//     config: &TlsConfig,
//     stream: &mut TcpStream,
// ) -> Result<mut TcpStream, Error> {
//     let certs =
//         CertificateDer::pem_file_iter(&config.certificate)?.collect::<Result<Vec<_>, _>>()?;
//     let key = PrivateKeyDer::from_pem_file(&config.private_key)?;
//
//     let config = rustls::ServerConfig::builder()
//         .with_no_client_auth()
//         .with_single_cert(certs, key)?;
//
//     let acceptor = TlsAcceptor::from(Arc::new(config));
//     // let tls_stream = acceptor.accept(stream).await?;
//     let tls_stream = acceptor.accept(stream).await?;
//     debug!("TLS negotiation complete");
//
//     // Extract the underlying stream
//     let stream = tls_stream.into_inner().0;
//
//     Ok(stream)
// }

pub async fn accept_tls_differently<T: AsyncRead + AsyncWrite + Unpin>(
    config: &TlsConfig,
    stream: T,
) -> Result<TlsStream<T>, Error> {
    let certs =
        CertificateDer::pem_file_iter(&config.certificate)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(&config.private_key)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    let acceptor = TlsAcceptor::from(Arc::new(config));
    let tls_stream = acceptor.accept(stream).await?;
    debug!("TLS negotiation complete");

    Ok(tls_stream)
}

#[derive(Clone, Debug)]
pub struct NoCertificateVerification {}

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}
