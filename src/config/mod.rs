mod dataset;
// mod dataset_manager;
mod tandem;

pub use dataset::JsonDatasetConfig;
pub use tandem::TandemConfig;

use crate::error::{ConfigError, Error};
use cipherstash_config::DatasetConfig;
use tokio_postgres::{Client, NoTls, SimpleQueryMessage, SimpleQueryRow};
use tracing::{error, warn};

pub const CS_PREFIX: &str = "CS";
pub const DEFAULT_CONFIG_FILE_PATH: &str = "cipherstash-proxy.toml";

const ENCRYPT_DATASET_CONFIG_QUERY: &'static str = include_str!("./sql/select_config.sql");

pub async fn connect(connection_string: String) -> Result<Client, tokio_postgres::Error> {
    let (client, connection) = tokio_postgres::connect(&connection_string, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    Ok(client)
}

pub async fn load_dataset_config(config: &TandemConfig) -> Result<DatasetConfig, Error> {
    let client = connect(config.connect.to_connection_string()).await?;
    let result = client.simple_query(ENCRYPT_DATASET_CONFIG_QUERY).await;

    warn!("result: {:?}", result);

    let rows = match result {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| match row {
                SimpleQueryMessage::Row(row) => Some(row),
                _ => None,
            })
            .collect::<Vec<SimpleQueryRow>>(),
        Err(e) => {
            if configuration_table_not_found(&e) {
                error!("No Encrypt configuration table in database.");
                warn!("Encrypt requires the Encrypt Query Language (EQL) to be installed in the target database");
                warn!("See https://github.com/cipherstash/encrypt-query-language");

                return Err(ConfigError::MissingEncryptConfigTable.into());
            }

            error!("Error loading Encrypt configuration");
            return Err(ConfigError::Database(e).into());
        }
    };

    if rows.is_empty() {
        error!("No active Encrypt configuration");
        return Err(ConfigError::MissingActiveEncryptConfig.into());
    };

    let data = rows
        .first()
        .ok_or_else(|| ConfigError::MissingActiveEncryptConfig)
        .and_then(|row| row.try_get(0).map_err(|e| ConfigError::Database(e)))
        .and_then(|opt_str: Option<&str>| {
            opt_str.ok_or_else(|| ConfigError::MissingActiveEncryptConfig)
        })?;

    let config: JsonDatasetConfig =
        serde_json::from_str(&data).map_err(|e| ConfigError::Parse(e))?;

    let dataset_config = config.try_into().map_err(|e| ConfigError::Dataset(e))?;

    Ok(dataset_config)
}

fn configuration_table_not_found(e: &tokio_postgres::Error) -> bool {
    let msg = e.to_string();
    msg.contains("cs_configuration_v1") && msg.contains("does not exist")
}
