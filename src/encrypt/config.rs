use arc_swap::ArcSwap;
use driver::schema::DatasetConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, trace, warn};

use crate::errors::Error;
use crate::pool::{get_all_pools, PoolIdentifier};
use crate::server::Server;

use super::postgres_encrypt_config::PostgresEncryptConfig;
use super::send_query::send_query;

const ENCRYPT_CONFIG_QUERY: &'static str = include_str!("./sql/select_config.sql");

#[derive(Debug, Error)]
pub enum EncryptConfigError {
    #[error("error parsing Encrypt config: {0}")]
    ParseError(String),

    #[error("expected one active Encrypt configuration, found {found}")]
    LoadError { found: usize },
}

type EncryptConfigsByDatabaseName = Arc<ArcSwap<HashMap<String, Arc<DatasetConfig>>>>;

pub struct EncryptConfigManagerOpts {
    pub refresh_duration_seconds: u64,
}

impl Default for EncryptConfigManagerOpts {
    fn default() -> Self {
        Self {
            refresh_duration_seconds: 60,
        }
    }
}

#[derive(Clone)]
pub struct EncryptConfigManager {
    encrypt_configs: EncryptConfigsByDatabaseName,
    #[allow(dead_code)]
    refresh_job: Option<Arc<JoinHandle<()>>>,
}

impl EncryptConfigManager {
    pub fn init_static(encrypt_configs: HashMap<String, Arc<DatasetConfig>>) -> Self {
        Self {
            encrypt_configs: Arc::new(ArcSwap::new(Arc::new(encrypt_configs))),
            refresh_job: None,
        }
    }

    pub fn init_empty() -> Self {
        Self::init_static(Default::default())
    }

    /// Get the encrypt_config for a specific database by name
    pub fn get_encrypt_config_for_database(&self, name: &str) -> Option<Arc<DatasetConfig>> {
        self.encrypt_configs.load().get(name).cloned()
    }

    pub async fn init() -> Result<Self, Error> {
        Self::init_with_opts(Default::default()).await
    }

    pub async fn init_with_opts(opts: EncryptConfigManagerOpts) -> Result<Self, Error> {
        let encrypt_configs = Self::fetch_encrypt_configs_with_retry().await?;
        let encrypt_configs = Arc::new(ArcSwap::new(Arc::new(encrypt_configs)));

        let refresh_job_encrypt_config_ref = encrypt_configs.clone();

        let refresh_job = Some(Arc::new(tokio::spawn(async move {
            let refresh_duration = tokio::time::Duration::from_secs(opts.refresh_duration_seconds);

            let mut interval = tokio::time::interval_at(
                tokio::time::Instant::now() + refresh_duration,
                refresh_duration,
            );

            loop {
                interval.tick().await;

                match Self::fetch_encrypt_configs().await {
                    Ok(new_encrypt_configs) => {
                        refresh_job_encrypt_config_ref.swap(Arc::new(new_encrypt_configs));
                    }

                    Err(e) => {
                        warn!("[cipherstash-proxy::encrypt_config] failed to fetch new encrypt configs");
                        trace!("[cipherstash-proxy::encrypt_config] with error: {e}");
                    }
                }
            }
        })));

        Ok(Self {
            encrypt_configs,
            refresh_job,
        })
    }

    pub async fn refresh_encrypt_configs(&self) {
        match Self::fetch_encrypt_configs().await {
            Ok(new_encrypt_configs) => {
                self.encrypt_configs.swap(Arc::new(new_encrypt_configs));
            }

            Err(e) => {
                warn!("[cipherstash-proxy::encrypt_config] failed to fetch new encrypt configs");
                trace!("[cipherstash-proxy::encrypt_config] with error: {e}");
            }
        }
    }

    async fn query_server_for_encrypt_config(server: &mut Server) -> Result<DatasetConfig, Error> {
        let mut encrypt_config_rows: Vec<String> = match send_query(ENCRYPT_CONFIG_QUERY, server)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                error!("Error loading Encrypt configuration");
                debug!("{e:?}");
                if configuration_table_not_found(&e) {
                    warn!("Encrypt requires the Encrypt Query Language (EQL) to be installed in the target Postgres database");
                    warn!("See https://github.com/cipherstash/encrypt-query-language");
                    vec![]
                } else {
                    return Err(e);
                }
            }
        };

        match encrypt_config_rows.len() {
            0 => {
                warn!("No active Encrypt configuration");
                Ok(DatasetConfig::init())
            }
            1 => {
                let encrypt_config = encrypt_config_rows.pop().unwrap();

                let encrypt_config = serde_json::from_str::<PostgresEncryptConfig>(&encrypt_config)
                    .map_err(|e| EncryptConfigError::ParseError(e.to_string()))?
                    .try_into()
                    .map_err(|e: driver::schema::errors::ConfigError| {
                        EncryptConfigError::ParseError(e.to_string())
                    })?;

                Ok(encrypt_config)
            }
            found => Err(EncryptConfigError::LoadError { found }.into()),
        }
    }

    /// Fetch the schema and retry on any error
    ///
    /// When databases and the proxy start up at the same time they might not be ready to accept connections before the
    /// proxy tries to query the schema. To give the proxy the best chance of initialising correctly this method will
    /// retry the query a few times before passing on the error.
    async fn fetch_encrypt_configs_with_retry() -> Result<HashMap<String, Arc<DatasetConfig>>, Error>
    {
        let mut retry_count = 0;
        let max_retry_count = 10;
        let max_backoff = Duration::from_secs(2);

        loop {
            match Self::fetch_encrypt_configs().await {
                Ok(value) => {
                    return Ok(value);
                }

                Err(e) => {
                    if retry_count >= max_retry_count {
                        return Err(e);
                    }
                }
            }

            let sleep_duration_ms =
                (100 * 2_u64.pow(retry_count)).min(max_backoff.as_millis() as _);

            tokio::time::sleep(Duration::from_millis(sleep_duration_ms)).await;

            retry_count += 1;
        }
    }

    /// Fetch the encrypt configs for all databases across all pools
    async fn fetch_encrypt_configs() -> Result<HashMap<String, Arc<DatasetConfig>>, Error> {
        info!("Fetching encrypt configs for all databases");

        let pools = get_all_pools();

        let mut databases = HashMap::new();

        for (PoolIdentifier { db, .. }, pool) in pools.into_iter() {
            // If the encrypt config for the database has already been fetched (maybe the database has multiple shards) just
            // skip fetching it again.
            if databases.contains_key(&db) {
                continue;
            }

            // Get an "out of band" connection to a healthy instance in the pool so server and client stats aren't incremented by this connection. This makes sure that calls like SHOW POOLS and SHOW CLIENTS don't show that the schema fetcher is connected.
            let mut connection = pool.get_dedicated_out_of_band_connection().await?;
            let server: &mut Server = &mut *connection;
            let encrypt_config = Self::query_server_for_encrypt_config(server).await?;
            databases.insert(db.to_string(), Arc::new(encrypt_config));
        }

        Ok(databases)
    }
}

fn configuration_table_not_found(e: &Error) -> bool {
    let msg = e.to_string();
    msg.contains("cs_configuration_v1") && msg.contains("does not exist")
}
