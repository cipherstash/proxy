#![allow(dead_code)]

use rand::{distributions::Alphanumeric, Rng};
use rustls::{
    client::danger::ServerCertVerifier, crypto::aws_lc_rs::default_provider,
    pki_types::CertificateDer, ClientConfig,
};
use std::sync::{Arc, Once};
use tokio_postgres::{Client, NoTls};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

pub const PROXY: u16 = 6432;
pub const PG_LATEST: u16 = 5532;
pub const PG_V17_TLS: u16 = 5617;

static INIT: Once = Once::new();

pub fn id() -> i64 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    rng.gen_range(1..=i64::MAX)
}

pub fn random_string() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(10) // Length of string
        .map(char::from)
        .collect()
}

pub async fn clear() {
    let client = connect_with_tls(PROXY).await;

    let sql = "TRUNCATE encrypted";
    client.simple_query(sql).await.expect("ok");
}

pub fn trace() {
    INIT.call_once(|| {
        let log_level: Directive = tracing::Level::DEBUG.into();

        let filter = EnvFilter::from_default_env().add_directive(log_level.to_owned());

        let subscriber = FmtSubscriber::builder()
            .with_env_filter(filter)
            .with_file(true)
            .with_line_number(true)
            .with_target(true)
            .finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    });
}

pub fn connection_string(port: u16) -> String {
    let host = "localhost".to_string();
    let name = "cipherstash".to_string();
    let username = "cipherstash".to_string();
    let password = "password".to_string();

    format!(
        "postgres://{}:{}@{}:{}/{}",
        username, password, host, port, name
    )
}

pub async fn connect_with_tls(port: u16) -> Client {
    let tls_config = configure_test_client();
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);

    let connection_string = connection_string(port);
    let (client, connection) = tokio_postgres::connect(&connection_string, tls)
        .await
        .expect("connection to database to succeed");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });
    client
}

pub async fn connect(port: u16) -> Client {
    let connection_string = connection_string(port);
    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
        .await
        .expect("ok");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    client
}

///
/// Configure the client TLS settings.
/// These are the settings for connecting to the database with TLS.
///
/// NOTE: This client uses a dangerous test certificate verifier that does not verify the server's certificate.
///
/// This is because the test database uses a self-signed certificate.
pub fn configure_test_client() -> ClientConfig {
    let verifier = DangerousTestCertVerifier;
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth()
}

/// Dangerous test certificate "verifier" that does not actually verify the server's certificate.
/// This **must** never be used for anything other than testing.
#[derive(Debug)]
struct DangerousTestCertVerifier;

impl ServerCertVerifier for DangerousTestCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
