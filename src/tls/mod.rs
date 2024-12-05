use crate::config::ServerConfig;
use crate::{config::TlsConfig, error::Error};
use crate::{DatabaseConfig, TandemConfig};
use rustls::client::danger::ServerCertVerifier;
use rustls::ClientConfig;
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};
use tracing::{debug, info, warn};

pub async fn client(
    stream: TcpStream,
    config: &TandemConfig,
) -> Result<TlsStream<TcpStream>, Error> {
    let tls_config = configure_client(&config.database);

    let connector = TlsConnector::from(Arc::new(tls_config));
    let domain = config.server.server_name()?.to_owned();
    let tls_stream = connector.connect(domain, stream).await?;

    Ok(tls_stream.into())
}

pub async fn server(stream: TcpStream, config: &TlsConfig) -> Result<TlsStream<TcpStream>, Error> {
    let certs =
        CertificateDer::pem_file_iter(&config.certificate)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(&config.private_key)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    let acceptor = TlsAcceptor::from(Arc::new(config));
    let tls_stream = acceptor.accept(stream).await?;
    Ok(tls_stream.into())
}

pub fn configure_client(config: &DatabaseConfig) -> ClientConfig {
    let mut root_cert_store = rustls::RootCertStore::empty();
    root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();

    if !config.with_tls_verification {
        let mut dangerous = tls_config.dangerous();
        dangerous.set_certificate_verifier(Arc::new(NoCertificateVerification {}));
    }

    tls_config
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
