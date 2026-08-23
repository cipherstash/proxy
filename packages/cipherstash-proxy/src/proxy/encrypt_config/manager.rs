use super::from_domain::column_config_from_domain;
use crate::{
    config::DatabaseConfig, connect, error::Error, log::ENCRYPT_CONFIG, proxy::SCHEMA_QUERY,
};
use arc_swap::ArcSwap;
use cipherstash_client::eql;
use cipherstash_client::schema::ColumnConfig;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time};
use tracing::{debug, error, info, warn};

///
/// Column configuration keyed by table name and column name
///    - key: `{table_name}.{column_name}`
///
type EncryptConfigMap = HashMap<eql::Identifier, ColumnConfig>;

#[derive(Clone, Debug, PartialEq)]
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

    /// Returns whether any encrypted column belongs to `table`.
    pub(crate) fn contains_table(&self, table: &str) -> bool {
        self.config
            .keys()
            .any(|identifier| identifier.table == table)
    }

    /// Inserts or replaces encryption metadata for one column.
    pub(crate) fn insert(&mut self, identifier: eql::Identifier, config: ColumnConfig) {
        self.config.insert(identifier, config);
    }

    /// Removes encryption metadata for one column.
    pub(crate) fn remove_column(&mut self, table: &str, column: &str) {
        self.config
            .remove(&eql::Identifier::new(table.to_owned(), column.to_owned()));
    }

    /// Removes all encryption metadata for a table.
    pub(crate) fn remove_table(&mut self, table: &str) {
        self.config
            .retain(|identifier, _| identifier.table != table);
    }

    /// Moves encryption metadata to a renamed column identifier.
    pub(crate) fn rename_column(&mut self, table: &str, from: &str, to: &str) {
        let from = eql::Identifier::new(table.to_owned(), from.to_owned());
        if let Some(config) = self.config.remove(&from) {
            self.config.insert(
                eql::Identifier::new(table.to_owned(), to.to_owned()),
                config,
            );
        }
    }

    /// Moves all encryption metadata to a renamed table identifier.
    pub(crate) fn rename_table(&mut self, from: &str, to: &str) {
        let renamed = self
            .config
            .iter()
            .filter(|(identifier, _)| identifier.table == from)
            .map(|(identifier, config)| (identifier.column.clone(), config.clone()))
            .collect::<Vec<_>>();
        self.remove_table(from);
        for (column, config) in renamed {
            self.config
                .insert(eql::Identifier::new(to.to_owned(), column), config);
        }
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

    pub async fn reload(&self) -> bool {
        match load_encrypt_config_with_retry(&self.config).await {
            Ok(reloaded) => {
                debug!(target: ENCRYPT_CONFIG, msg = "Reloaded encrypt configuration");
                self.encrypt_config.swap(Arc::new(reloaded));
                true
            }
            Err(err) => {
                warn!(
                    msg = "Error reloading encrypt configuration",
                    error = err.to_string()
                );
                false
            }
        }
    }
}

async fn init_reloader(config: DatabaseConfig) -> Result<EncryptConfigManager, Error> {
    // Skip retries on startup as the likely failure mode is configuration
    // Only warn on startup, otherwise warning on every reload
    let encrypt_config = match load_encrypt_config(&config).await {
        Ok(encrypt_config) => encrypt_config,
        Err(err) => {
            // Encrypt config is inferred from the schema (EQL v3 self-configuring
            // domains), so a load error here is a database/connection failure, not
            // a missing config table. A schema with no encrypted columns is a
            // successful (empty) load, warned about below.
            error!(
                msg = "Error loading Encrypt configuration",
                error = err.to_string()
            );
            return Err(err);
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

/// Loads the encrypt configuration by inferring it from the database schema.
///
/// EQL v3 columns are self-configuring domain types, so each encrypted column's
/// `ColumnConfig` is derived from its Postgres domain typname (see
/// [`column_config_from_domain`]). There is no `eql_v2_configuration` table or
/// `add_search_config` in v3 — the schema is the single source of truth.
pub async fn load_encrypt_config(config: &DatabaseConfig) -> Result<EncryptConfig, Error> {
    let client = connect::database(config).await?;

    let tables = client.query(SCHEMA_QUERY, &[]).await?;

    let mut map = EncryptConfigMap::new();

    for table in tables {
        let table_name: String = table.get("table_name");
        let columns: Vec<String> = table.get("columns");
        let column_domain_names: Vec<Option<String>> = table.get("column_domain_names");

        for (column, domain) in columns.iter().zip(column_domain_names) {
            let Some(domain) = domain else { continue };
            if let Some(column_config) = column_config_from_domain(&table_name, column, &domain) {
                debug!(
                    target: ENCRYPT_CONFIG,
                    msg = "Encrypted column",
                    table = table_name,
                    column = column,
                    domain = domain
                );
                // First wins. The query returns the search path in precedence
                // order, so if a table name does appear in more than one
                // searched schema this keeps the one PostgreSQL itself would
                // resolve to, rather than whichever happened to come last.
                map.entry(eql::Identifier::new(table_name.clone(), column.clone()))
                    .or_insert(column_config);
            }
        }
    }

    Ok(EncryptConfig::new_from_config(map))
}
