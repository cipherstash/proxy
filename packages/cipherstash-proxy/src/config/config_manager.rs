use super::{tandem::DatabaseConfig, EncryptConfig};
use crate::{
    config::ENCRYPT_CONFIG_QUERY,
    connect, eql,
    error::{ConfigError, Error},
    log::DEVELOPMENT,
};
use arc_swap::ArcSwap;
use cipherstash_config::ColumnConfig;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time};
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

    pub fn is_empty(&self) -> bool {
        self.config.load().is_empty()
    }
}

async fn init_reloader(config: DatabaseConfig) -> Result<EncryptConfigManager, Error> {
    // Skip retries on startup as the likely failure mode is configuration
    // Only warn on startup, otherwise warning on every reload
    let encrypt_config = match load_encrypt_config(&config).await {
        Ok(encrypt_config) => encrypt_config,
        Err(err) => {
            match err {
                // Similar messages are displayed on connection, defined in handler.rs
                // Please keep the language in sync when making changes here.
                Error::Config(ConfigError::MissingEncryptConfigTable) => {
                    error!(msg = "No Encrypt configuration table in database.");
                    warn!(msg = "Encrypt requires the Encrypt Query Language (EQL) to be installed in the target database");
                    warn!(msg = "See https://github.com/cipherstash/encrypt-query-language");
                }
                _ => {
                    error!(
                        msg = "Error loading Encrypt configuration",
                        error = err.to_string()
                    );
                    return Err(err);
                }
            }
            HashMap::new()
        }
    };

    if encrypt_config.is_empty() {
        warn!(msg = "⚠️ ENCRYPT CONFIGURATION IS EMPTY");
    } else {
        info!(msg = "Loaded Encrypt configuration");
    }

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

            match load_encrypt_config_with_retry(&config_ref).await {
                Ok(reloaded) => {
                    debug!(target: DEVELOPMENT, msg = "Reloaded Encrypt configuration");
                    dataset_ref.swap(Arc::new(reloaded));
                }
                Err(err) => {
                    warn!(
                        msg = "Error reloading Encrypt configuration",
                        error = err.to_string()
                    );
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
async fn load_encrypt_config_with_retry(
    config: &DatabaseConfig,
) -> Result<EncryptConfigMap, Error> {
    let mut retry_count = 0;
    let max_retry_count = 10;
    let max_backoff = Duration::from_secs(2);

    loop {
        match load_encrypt_config(config).await {
            Ok(encrypt_config) => {
                return Ok(encrypt_config);
            }

            Err(err) => {
                if retry_count >= max_retry_count {
                    debug!(
                        DEVELOPMENT,
                        msg = "Encrypt configuration could not beloaded",
                        retries = retry_count,
                        error = err.to_string()
                    );
                    return Err(err);
                }
            }
        }

        let sleep_duration_ms = (100 * 2_u64.pow(retry_count)).min(max_backoff.as_millis() as _);

        time::sleep(Duration::from_millis(sleep_duration_ms)).await;

        retry_count += 1;
    }
}

pub async fn load_encrypt_config(config: &DatabaseConfig) -> Result<EncryptConfigMap, Error> {
    let client = connect::database(config).await?;

    match client.query(ENCRYPT_CONFIG_QUERY, &[]).await {
        Ok(rows) => {
            if rows.is_empty() {
                return Ok(EncryptConfigMap::new());
            };

            // We know there is at least one row
            let row = rows.first().unwrap();

            let json_value: Value = row.get("data");
            let encrypt_config: EncryptConfig = serde_json::from_value(json_value)?;
            Ok(encrypt_config.to_config_map())
        }
        Err(err) => {
            if configuration_table_not_found(&err) {
                return Err(ConfigError::MissingEncryptConfigTable.into());
            }
            Err(ConfigError::Database(err).into())
        }
    }
}

fn configuration_table_not_found(e: &tokio_postgres::Error) -> bool {
    let msg = e.to_string();
    msg.contains("cs_configuration_v1") && msg.contains("does not exist")
}
