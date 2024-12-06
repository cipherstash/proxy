use cipherstash_proxy::{config::DatabaseConfig, tls};
use tokio_postgres::{Client, NoTls};

pub const PROXY: u16 = 6432;
pub const PG_LATEST: u16 = 5532;
pub const PG_v17_TLS: u16 = 5617;

pub fn database_config() -> DatabaseConfig {
    database_config_with_port(PG_LATEST)
}

pub fn database_config_with_port(port: u16) -> DatabaseConfig {
    DatabaseConfig {
        host: "localhost".to_string(),
        port: port,
        name: "cipherstash".to_string(),
        username: "cipherstash".to_string(),
        password: "password".to_string(),
        config_reload_interval: 10,
        schema_reload_interval: 10,
        with_tls_verification: false,
    }
}

pub async fn connect_with_tls(config: &DatabaseConfig) -> Client {
    let tls_config = tls::configure_client(&config);
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);

    let connection_string = config.to_connection_string();

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

pub async fn connect(config: &DatabaseConfig) -> Client {
    let connection_string = config.to_connection_string();

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
