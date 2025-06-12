#![allow(dead_code)]

use rand::{distr::Alphanumeric, Rng};
use rustls::{
    client::danger::ServerCertVerifier, crypto::aws_lc_rs::default_provider,
    pki_types::CertificateDer, ClientConfig,
};
use std::sync::{Arc, Once};
use tokio_postgres::{types::ToSql, Client, NoTls};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

pub const PROXY: u16 = 6432;
pub const PG_LATEST: u16 = 5532;
pub const PG_V17_TLS: u16 = 5617;

pub const TEST_SCHEMA_SQL: &str = include_str!(concat!("../../../tests/sql/schema.sql"));

static INIT: Once = Once::new();

pub fn random_id() -> i64 {
    use rand::Rng;
    let mut rng = rand::rng();
    rng.random_range(1..=i64::MAX)
}

// Limited by valid data range
pub fn random_limited() -> i32 {
    use rand::Rng;
    let mut rng = rand::rng();
    rng.random_range(1..=31)
}

pub fn random_string() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(10) // Length of string
        .map(char::from)
        .collect()
}

pub async fn clear() {
    let client = connect_with_tls(PROXY).await;

    let sql = "TRUNCATE encrypted";
    client.simple_query(sql).await.unwrap();

    let sql = "TRUNCATE plaintext";
    client.simple_query(sql).await.unwrap();
}

pub async fn reset_schema() {
    let port = std::env::var("CS_DATABASE__PORT")
        .map(|s| s.parse().unwrap())
        .unwrap_or(PG_LATEST);

    let client = connect_with_tls(port).await;
    client.simple_query(TEST_SCHEMA_SQL).await.unwrap();
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

pub fn connection_config(port: u16) -> tokio_postgres::Config {
    let mut db_config = tokio_postgres::Config::new();

    let host = "localhost".to_string();
    let name = "cipherstash".to_string();
    let username = "cipherstash".to_string();
    let password = "p@ssword".to_string();

    db_config
        .host(&host)
        .port(port)
        .user(&username)
        .password(&password)
        .dbname(&name);

    db_config
}

pub async fn connect_with_tls(port: u16) -> Client {
    let tls_config = configure_test_client();
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);

    let connection_config = connection_config(port);
    let (client, connection) = connection_config
        .connect(tls)
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
    let connection_config = connection_config(port);
    let (client, connection) = connection_config.connect(NoTls).await.unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    client
}

pub async fn insert(sql: &str, params: &[&(dyn ToSql + Sync)]) {
    let client = connect_with_tls(PROXY).await;
    client.query(sql, params).await.unwrap();
}

pub async fn insert_simple_query(sql: &str) {
    let client = connect_with_tls(PROXY).await;
    client.simple_query(sql).await.unwrap();
}

pub async fn query<T: for<'a> tokio_postgres::types::FromSql<'a> + Send + Sync>(
    sql: &str,
) -> Vec<T> {
    let client = connect_with_tls(PROXY).await;
    let rows = client.query(sql, &[]).await.unwrap();
    rows.iter().map(|row| row.get(0)).collect::<Vec<T>>()
}

pub async fn query_by<T>(sql: &str, param: &(dyn ToSql + Sync)) -> Vec<T>
where
    T: for<'a> tokio_postgres::types::FromSql<'a> + Send + Sync,
{
    let client = connect_with_tls(PROXY).await;
    let rows = client.query(sql, &[param]).await.unwrap();
    rows.iter().map(|row| row.get(0)).collect::<Vec<T>>()
}

pub async fn simple_query<T: std::str::FromStr>(sql: &str) -> Vec<T>
where
    <T as std::str::FromStr>::Err: std::fmt::Debug,
{
    let client = connect_with_tls(PROXY).await;
    let rows = client.simple_query(sql).await.unwrap();
    rows.iter()
        .filter_map(|row| {
            if let tokio_postgres::SimpleQueryMessage::Row(r) = row {
                r.get(0).and_then(|val| val.parse::<T>().ok())
            } else {
                None
            }
        })
        .collect()
}

// Returns a vector of `Option<String>` for each row in the result set.
// Nulls are represented as `None`, and non-null values are converted to `Some(String)`.
pub async fn simple_query_with_null(sql: &str) -> Vec<Option<String>> {
    let client = connect_with_tls(PROXY).await;
    let rows = client.simple_query(sql).await.unwrap();
    rows.iter()
        .filter_map(|row| {
            if let tokio_postgres::SimpleQueryMessage::Row(r) = row {
                Some(r.get(0).map(|val| val.to_string()))
            } else {
                None
            }
        })
        .collect()
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
