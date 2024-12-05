use super::tandem::DatabaseConfig;
use crate::{
    config::{EncryptConfig, ENCRYPT_DATASET_CONFIG_QUERY},
    connect, eql,
    error::{ConfigError, Error},
};
use arc_swap::ArcSwap;
use cipherstash_config::ColumnConfig;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time};
use tokio_postgres::{SimpleQueryMessage, SimpleQueryRow};
use tracing::{debug, error, info, warn};

///
/// Column configuration keyed by table name and column name
///    - key: `{table_name}.{column_name}`
///
type EncryptConfigMap = HashMap<eql::Identifier, ColumnConfig>;

#[derive(Clone, Debug)]
pub struct EncryptConfigManager {
    _reload_handle: Arc<JoinHandle<()>>,
    config: Arc<ArcSwap<EncryptConfigMap>>,
}

impl EncryptConfigManager {
    pub async fn init(config: &DatabaseConfig) -> Result<Self, Error> {
        let config = config.clone();
        init_reloader(config).await
    }

    pub fn load(&self) -> Arc<EncryptConfigMap> {
        self.config.load().clone()
    }
}

async fn init_reloader(config: DatabaseConfig) -> Result<EncryptConfigManager, Error> {
    // Skip retries on startup as the likely failure mode is configuration
    let encrypt_config = load_dataset(&config).await?;
    info!("Loaded Encrypt configuration");

    let encrypt_config = Arc::new(ArcSwap::new(Arc::new(encrypt_config)));

    let config_ref = config.clone();

    let dataset_ref = encrypt_config.clone();
    let reload_handle = tokio::spawn(async move {
        let reload_interval = tokio::time::Duration::from_secs(config_ref.config_reload_interval);

        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + reload_interval,
            reload_interval,
        );

        loop {
            interval.tick().await;

            match load_dataset(&config_ref).await {
                Ok(reloaded) => {
                    // debug!("Reloaded Encrypt configuration");
                    dataset_ref.swap(Arc::new(reloaded));
                }
                Err(e) => {
                    warn!("Error reloading Encrypt configuration");
                    warn!("{e}");
                }
            }
        }
    });

    Ok(EncryptConfigManager {
        config: encrypt_config,
        _reload_handle: Arc::new(reload_handle),
    })
}

/// Fetch the dataset and retry on any error
///
/// When databases and the proxy start up at the same time they might not be ready to accept connections before the
/// proxy tries to query the schema. To give the proxy the best chance of initialising correctly this method will
/// retry the query a few times before passing on the error.
async fn load_dataset_with_retry(config: &DatabaseConfig) -> Result<EncryptConfigMap, Error> {
    let mut retry_count = 0;
    let max_retry_count = 10;
    let max_backoff = Duration::from_secs(2);

    loop {
        match load_dataset(config).await {
            Ok(encrypt_config) => {
                return Ok(encrypt_config);
            }

            Err(e) => {
                if retry_count >= max_retry_count {
                    return Err(e);
                }
            }
        }

        let sleep_duration_ms = (100 * 2_u64.pow(retry_count)).min(max_backoff.as_millis() as _);

        time::sleep(Duration::from_millis(sleep_duration_ms)).await;

        retry_count += 1;
    }
}

pub async fn load_dataset(config: &DatabaseConfig) -> Result<EncryptConfigMap, Error> {
    let client = connect::database(config).await?;

    let result = client.simple_query(ENCRYPT_DATASET_CONFIG_QUERY).await;

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

    let encrypt = EncryptConfig::from_str(&data)?;
    let map = encrypt.to_config_map();

    Ok(map)
}

fn configuration_table_not_found(e: &tokio_postgres::Error) -> bool {
    let msg = e.to_string();
    msg.contains("cs_configuration_v1") && msg.contains("does not exist")
}
