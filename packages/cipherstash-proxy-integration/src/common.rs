#![allow(dead_code)]

use rand::{distributions::Alphanumeric, Rng};
use rustls::ClientConfig;
use std::sync::Once;
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
    let tls_config = configure_client();
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);

    let connection_string = connection_string(port);
    let (client, connection) = tokio_postgres::connect(&connection_string, tls)
        .await
        .expect("ok");

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
/// Configure the client TLS settings
/// These are the settings for connecting to the database with TLS
/// The client will use the system root certificates
///
pub fn configure_client() -> ClientConfig {
    let mut root_cert_store = rustls::RootCertStore::empty();
    root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    rustls::ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth()
}
