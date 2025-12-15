#![allow(dead_code)]

use rand::{distr::Alphanumeric, Rng};
use rustls::{
    client::danger::ServerCertVerifier, crypto::aws_lc_rs::default_provider,
    pki_types::CertificateDer, ClientConfig,
};
use serde_json::Value;
use std::sync::{Arc, Once};
use tokio_postgres::{types::ToSql, Client, NoTls, Row, SimpleQueryMessage};
use tracing::info;
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

pub const PROXY: u16 = 6432;
pub const PG_PORT: u16 = 5532;
pub const PG_TLS_PORT: u16 = 5617;

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
        .unwrap_or(PG_PORT);

    let client = connect_with_tls(port).await;
    client.simple_query(TEST_SCHEMA_SQL).await.unwrap();
}

pub async fn reset_schema_to(schema: &'static str) {
    let port = std::env::var("CS_DATABASE__PORT")
        .map(|s| s.parse().unwrap())
        .unwrap_or(PG_PORT);

    let client = connect_with_tls(port).await;
    client.simple_query(schema).await.unwrap();
}

pub async fn table_exists(table: &str) -> bool {
    let query = format!(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM information_schema.tables
            WHERE table_schema = 'public'
            AND table_name = '{table}'
        );
    "#
    );

    let port = std::env::var("CS_DATABASE__PORT")
        .map(|s| s.parse().unwrap())
        .unwrap_or(PG_PORT);

    let client = connect_with_tls(port).await;
    let messages = client.simple_query(&query).await.unwrap();

    for message in messages {
        if let SimpleQueryMessage::Row(row) = message {
            return row.get(0) == Some("t");
        }
    }

    false
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
            eprintln!("connection error: {e}");
        }
    });
    client
}

pub async fn connect(port: u16) -> Client {
    let connection_config = connection_config(port);
    let (client, connection) = connection_config.connect(NoTls).await.unwrap();

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {e}");
        }
    });

    client
}

pub async fn execute_query(sql: &str, params: &[&(dyn ToSql + Sync)]) {
    let client = connect_with_tls(PROXY).await;
    client.query(sql, params).await.unwrap();
}

pub async fn execute_simple_query(sql: &str) {
    let client = connect_with_tls(PROXY).await;
    client.simple_query(sql).await.unwrap();
}

pub async fn query<T: for<'a> tokio_postgres::types::FromSql<'a> + Send + Sync>(
    sql: &str,
) -> Vec<T> {
    let client = connect_with_tls(PROXY).await;
    query_with_client(sql, &client).await
}

pub async fn query_with_client<T: for<'a> tokio_postgres::types::FromSql<'a> + Send + Sync>(
    sql: &str,
    client: &Client,
) -> Vec<T> {
    let rows = client.query(sql, &[]).await.unwrap();
    rows.iter().map(|row| row.get(0)).collect::<Vec<T>>()
}

pub fn rows_to_vec<T: for<'a> tokio_postgres::types::FromSql<'a> + Send + Sync>(
    rows: &[Row],
) -> Vec<T> {
    rows.iter().map(|row| row.get(0)).collect::<Vec<T>>()
}

pub async fn query_by<T>(sql: &str, param: &(dyn ToSql + Sync)) -> Vec<T>
where
    T: for<'a> tokio_postgres::types::FromSql<'a> + Send + Sync,
{
    query_by_params(sql, &[param]).await
}

pub async fn query_by_params<T>(sql: &str, params: &[&(dyn ToSql + Sync)]) -> Vec<T>
where
    T: for<'a> tokio_postgres::types::FromSql<'a> + Send + Sync,
{
    let client = connect_with_tls(PROXY).await;
    let rows = client.query(sql, params).await.unwrap();
    rows.iter().map(|row| row.get(0)).collect::<Vec<T>>()
}

/// Get database port from environment or use default.
fn get_database_port() -> u16 {
    std::env::var("CS_DATABASE__PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(PG_PORT)
}

pub async fn query_direct_by<T>(sql: &str, param: &(dyn ToSql + Sync)) -> Vec<T>
where
    T: for<'a> tokio_postgres::types::FromSql<'a>,
{
    let port = get_database_port();
    info!(port);

    let client = connect_with_tls(port).await;
    let rows = client.query(sql, &[param]).await.unwrap();
    rows.iter().map(|row| row.get(0)).collect()
}

pub async fn simple_query<T: std::str::FromStr>(sql: &str) -> Vec<T>
where
    <T as std::str::FromStr>::Err: std::fmt::Debug,
{
    let client = connect_with_tls(PROXY).await;

    simple_query_with_client(sql, &client).await
}

pub async fn simple_query_with_client<T: std::str::FromStr>(sql: &str, client: &Client) -> Vec<T>
where
    <T as std::str::FromStr>::Err: std::fmt::Debug,
{
    let rows = client.simple_query(sql).await.unwrap();
    rows.iter()
        .filter_map(|row| {
            if let SimpleQueryMessage::Row(r) = row {
                r.get(0).and_then(|val| {
                    // Convert string value to FromSql compatible type
                    // Try different type conversions based on the value format
                    // PostgreSQL returns booleans as "t" or "f" in simple queries

                    // Convert PostgreSQL boolean format to native rust representation
                    match val {
                        "t" => "true".parse::<T>().ok(),
                        "f" => "false".parse::<T>().ok(),
                        _ => val.parse::<T>().ok(),
                    }
                })
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
            if let SimpleQueryMessage::Row(r) = row {
                Some(r.get(0).map(|val| val.to_string()))
            } else {
                None
            }
        })
        .collect()
}

pub async fn insert(sql: &str, params: &[&(dyn ToSql + Sync)]) {
    let client = connect_with_tls(PROXY).await;
    client.query(sql, params).await.unwrap();
}

pub async fn insert_jsonb() -> Value {
    let id = random_id();

    let encrypted_jsonb = serde_json::json!({
        "id": id,
        "string": "hello",
        "number": 42,
        "nested": {
            "number": 1815,
            "string": "world",
        },
        "array_string": ["hello", "world"],
        "array_number": [42, 84],
    });

    let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)".to_string();

    insert(&sql, &[&id, &encrypted_jsonb]).await;

    encrypted_jsonb
}

pub async fn insert_jsonb_for_search() {
    for n in 1..=5 {
        let id = random_id();
        let s = ((b'A' + (n - 1) as u8) as char).to_string();

        let encrypted_jsonb = serde_json::json!({
            "string": s,
            "number": n,
        });

        let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
        insert(sql, &[&id, &encrypted_jsonb]).await;
    }
}

/// Verifies that a text value was actually encrypted in the database.
/// Queries directly (bypassing proxy) and asserts stored value differs from plaintext.
pub async fn assert_encrypted_text(id: i64, column: &str, plaintext: &str) {
    let sql = format!("SELECT {}::text FROM encrypted WHERE id = $1", column);
    let stored: Vec<String> = query_direct_by(&sql, &id).await;

    assert_eq!(stored.len(), 1, "Expected exactly one row");
    let stored_text = &stored[0];

    assert_ne!(
        stored_text, plaintext,
        "ENCRYPTION FAILED for {}: Stored value matches plaintext! Data was not encrypted.",
        column
    );
}

///
/// Configure the client TLS settings.
/// These are the settings for connecting to the database with TLS.
///
/// NOTE: This client uses a dangerous test certificate verifier that does not verify the server's certificate.
///
/// This is because the test database uses a self-signed certificate.
pub fn configure_test_client() -> ClientConfig {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

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
