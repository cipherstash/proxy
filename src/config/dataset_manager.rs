use super::{connect, tandem::DatabaseConfig};
use crate::{
    config::{JsonDatasetConfig, ENCRYPT_DATASET_CONFIG_QUERY},
    error::{ConfigError, Error},
};
use arc_swap::ArcSwap;
use cipherstash_config::DatasetConfig;
use std::{sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time};
use tokio_postgres::{SimpleQueryMessage, SimpleQueryRow};
use tracing::{debug, error, warn};

#[derive(Clone, Debug)]
pub struct DatasetManager {
    _reload_handle: Arc<JoinHandle<()>>,
    dataset: Arc<ArcSwap<DatasetConfig>>,
}

impl DatasetManager {
    pub async fn init(config: &DatabaseConfig) -> Result<Self, Error> {
        let config = config.clone();
        init_reloader(config).await
    }

    pub fn load(&self) -> Arc<DatasetConfig> {
        self.dataset.load().clone()
    }
}

async fn init_reloader(config: DatabaseConfig) -> Result<DatasetManager, Error> {
    let dataset = load_dataset_with_retry(&config).await?;
    let dataset = Arc::new(ArcSwap::new(Arc::new(dataset)));

    let config_ref = config.clone();

    let dataset_ref = dataset.clone();
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
                    debug!("Reloaded Encrypt configuration");
                    dataset_ref.swap(Arc::new(reloaded));
                }
                Err(e) => {
                    warn!("Error reloading Encrypt configuration");
                    warn!("{e}");
                }
            }
        }
    });

    Ok(DatasetManager {
        dataset,
        _reload_handle: Arc::new(reload_handle),
    })
}

/// Fetch the dataset and retry on any error
///
/// When databases and the proxy start up at the same time they might not be ready to accept connections before the
/// proxy tries to query the schema. To give the proxy the best chance of initialising correctly this method will
/// retry the query a few times before passing on the error.
async fn load_dataset_with_retry(config: &DatabaseConfig) -> Result<DatasetConfig, Error> {
    let mut retry_count = 0;
    let max_retry_count = 10;
    let max_backoff = Duration::from_secs(2);

    loop {
        match load_dataset(config).await {
            Ok(dataset_config) => {
                return Ok(dataset_config);
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

pub async fn load_dataset(config: &DatabaseConfig) -> Result<DatasetConfig, Error> {
    return Ok(DatasetConfig::init());

    let client = connect(config.to_connection_string()).await?;
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

    let config: JsonDatasetConfig =
        serde_json::from_str(&data).map_err(|e| ConfigError::Parse(e))?;

    let dataset_config = config.try_into().map_err(|e| ConfigError::Dataset(e))?;

    Ok(dataset_config)
}

fn configuration_table_not_found(e: &tokio_postgres::Error) -> bool {
    let msg = e.to_string();
    msg.contains("cs_configuration_v1") && msg.contains("does not exist")
}
