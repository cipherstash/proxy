use crate::{
    config::DatabaseConfig,
    connect, eql,
    error::{ConfigError, Error},
    log::ENCRYPT_CONFIG,
    proxy::ENCRYPT_CONFIG_QUERY,
};
use arc_swap::ArcSwap;
use cipherstash_client::schema::ColumnConfig;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time};
use tracing::{debug, error, info, warn};

use super::config::ColumnEncryptionConfig;

///
/// Column configuration keyed by table name and column name
///    - key: `{table_name}.{column_name}`
///
type EncryptConfigMap = HashMap<eql::Identifier, ColumnConfig>;

#[derive(Clone, Debug)]
pub struct EncryptConfig {
    config: EncryptConfigMap,
}

impl EncryptConfig {
    pub fn new_from_config(config: EncryptConfigMap) -> Self {
        Self { config }
    }

    pub fn new() -> Self {
        Self {
            config: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.config.is_empty()
    }

    pub fn get_column_config(&self, identifier: &eql::Identifier) -> Option<ColumnConfig> {
        self.config.get(identifier).cloned()
    }
}

impl Default for EncryptConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct EncryptConfigManager {
    config: DatabaseConfig,
    encrypt_config: Arc<ArcSwap<EncryptConfig>>,
    _reload_handle: Arc<JoinHandle<()>>,
}

impl EncryptConfigManager {
    pub async fn init(config: &DatabaseConfig) -> Result<Self, Error> {
        let config = config.clone();
        init_reloader(config).await
    }

    pub fn load(&self) -> Arc<EncryptConfig> {
        self.encrypt_config.load().clone()
    }

    pub fn is_empty(&self) -> bool {
        self.encrypt_config.load().is_empty()
    }

    pub async fn reload(&self) {
        match load_encrypt_config_with_retry(&self.config).await {
            Ok(reloaded) => {
                debug!(target: ENCRYPT_CONFIG, msg = "Reloaded encrypt configuration");
                self.encrypt_config.swap(Arc::new(reloaded));
            }
            Err(err) => {
                warn!(
                    msg = "Error reloading encrypt configuration",
                    error = err.to_string()
                );
            }
        };
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
            EncryptConfig::new()
        }
    };

    debug!(target: ENCRYPT_CONFIG, ?encrypt_config);

    if encrypt_config.is_empty() {
        warn!(msg = "ENCRYPT CONFIGURATION NOT LOADED");
        warn!(msg = "No active Encrypt configuration found in database.");
        warn!(msg = "Data is not protected with encryption");
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
                    debug!(target: ENCRYPT_CONFIG, msg = "Reloaded Encrypt configuration");
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
        config,
        encrypt_config,
        _reload_handle: Arc::new(reload_handle),
    })
}

/// Fetch the dataset and retry on any error
///
/// When databases and the proxy start up at the same time they might not be ready to accept connections before the
/// proxy tries to query the schema. To give the proxy the best chance of initialising correctly this method will
/// retry the query a few times before passing on the error.
async fn load_encrypt_config_with_retry(config: &DatabaseConfig) -> Result<EncryptConfig, Error> {
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
                        ENCRYPT_CONFIG,
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

pub async fn load_encrypt_config(config: &DatabaseConfig) -> Result<EncryptConfig, Error> {
    let client = connect::database(config).await?;

    match client.query(ENCRYPT_CONFIG_QUERY, &[]).await {
        Ok(rows) => {
            if rows.is_empty() {
                return Ok(EncryptConfig::new());
            };

            // We know there is at least one row
            let row = rows.first().unwrap();

            let json_value: Value = row.get("data");
            let encrypt_config: ColumnEncryptionConfig = serde_json::from_value(json_value)?;
            let encrypt_config = EncryptConfig::new_from_config(encrypt_config.into_config_map());

            Ok(encrypt_config)
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
    msg.contains("eql_v2_configuration") && msg.contains("does not exist")
}
