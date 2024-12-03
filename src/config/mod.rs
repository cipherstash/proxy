mod dataset;
mod dataset_manager;
mod schema_manager;
mod tandem;

pub use dataset::JsonDatasetConfig;
pub use dataset_manager::DatasetManager;
pub use schema_manager::SchemaManager;
pub use tandem::{DatabaseConfig, ServerConfig, TandemConfig, TlsConfig};
use tokio_postgres::{Client, NoTls};
use tracing::debug;

pub const CS_PREFIX: &str = "CS";
pub const DEFAULT_CONFIG_FILE_PATH: &str = "cipherstash-proxy.toml";

const ENCRYPT_DATASET_CONFIG_QUERY: &'static str = include_str!("./sql/select_config.sql");

const SCHEMA_QUERY: &'static str = include_str!("./sql/select_table_schemas.sql");

const AGGREGATE_QUERY: &'static str = include_str!("./sql/select_aggregates.sql");

pub async fn connect(config: &DatabaseConfig) -> Result<Client, tokio_postgres::Error> {
    let connection_string = config.to_connection_string();
    debug!("connection_string: {connection_string}");

    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    Ok(client)
}
