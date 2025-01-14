use super::{tandem::DatabaseConfig, AGGREGATE_QUERY, SCHEMA_QUERY};
use crate::error::Error;
use crate::{connect, log::SCHEMA};
use arc_swap::ArcSwap;
use eql_mapper::{Column, Schema, Table};
use sqlparser::ast::Ident;
use std::sync::Arc;
use std::time::Duration;
use tokio::{task::JoinHandle, time};
use tracing::{debug, info, warn};

#[derive(Clone, Debug)]
pub struct SchemaManager {
    config: DatabaseConfig,
    schema: Arc<ArcSwap<Schema>>,
    _reload_handle: Arc<JoinHandle<()>>,
}

impl SchemaManager {
    pub async fn init(config: &DatabaseConfig) -> Result<Self, Error> {
        let config = config.clone();
        init_reloader(config).await
    }

    pub fn load(&self) -> Arc<Schema> {
        self.schema.load().clone()
    }

    pub async fn reload(&self) {
        match load_schema_with_retry(&self.config).await {
            Ok(reloaded) => {
                debug!(target = SCHEMA, "Reloaded database schema");
                self.schema.swap(Arc::new(reloaded));
            }
            Err(e) => {
                warn!("Error reloading Encrypt configuration");
                warn!("{e}");
            }
        };
    }
}

async fn init_reloader(config: DatabaseConfig) -> Result<SchemaManager, Error> {
    // Skip retries on startup as the likely failure mode is configuration
    let schema = load_schema(&config).await?;
    info!("Loaded database schema");

    let schema = Arc::new(ArcSwap::new(Arc::new(schema)));

    let config_ref = config.clone();
    let schema_ref = schema.clone();

    let reload_handle = tokio::spawn(async move {
        let reload_interval = tokio::time::Duration::from_secs(config_ref.config_reload_interval);

        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + reload_interval,
            reload_interval,
        );

        loop {
            interval.tick().await;

            match load_schema_with_retry(&config_ref).await {
                Ok(reloaded) => {
                    // debug!("Reloaded database schema");
                    schema_ref.swap(Arc::new(reloaded));
                }
                Err(e) => {
                    warn!("Error reloading Encrypt configuration");
                    warn!("{e}");
                }
            }
        }
    });

    Ok(SchemaManager {
        config,
        schema,
        _reload_handle: Arc::new(reload_handle),
    })
}

/// Fetch the dataset and retry on any error
///
/// When databases and the proxy start up at the same time they might not be ready to accept connections before the
/// proxy tries to query the schema. To give the proxy the best chance of initialising correctly this method will
/// retry the query a few times before passing on the error.
async fn load_schema_with_retry(config: &DatabaseConfig) -> Result<Schema, Error> {
    let mut retry_count = 0;
    let max_retry_count = 10;
    let max_backoff = Duration::from_secs(2);

    loop {
        match load_schema(config).await {
            Ok(schema) => {
                return Ok(schema);
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

pub async fn load_schema(config: &DatabaseConfig) -> Result<Schema, Error> {
    let client = connect::database(config).await?;

    let tables = client.query(SCHEMA_QUERY, &[]).await?;

    let mut schema = Schema::new("public");

    if tables.is_empty() {
        warn!("Database schema contains no tables");
        return Ok(schema);
    };

    for table in tables {
        let table_name: String = table.get("table_name");
        let primary_keys: Vec<Option<String>> = table.get("primary_keys");
        let columns: Vec<String> = table.get("columns");
        let _types: Vec<Option<String>> = table.get("column_types");
        let domains: Vec<Option<String>> = table.get("column_domains");

        let mut table = Table::new(Ident::new(&table_name));

        columns.iter().zip(domains).for_each(|(col, domain)| {
            let is_primary_key = primary_keys.contains(&Some(col.to_string()));

            let ident = Ident::with_quote('"', col);

            let column = match domain.as_deref() {
                Some("cs_encrypted_v1") => {
                    debug!(target: SCHEMA, "cs_encrypted_v1 column: {table_name}.{col}");
                    Column::eql(ident)
                }
                _ => Column::native(ident),
            };

            table.add_column(Arc::new(column), is_primary_key);
        });

        schema.add_table(table);
    }

    let aggregates = client.query(AGGREGATE_QUERY, &[]).await?;
    schema.aggregates = aggregates
        .into_iter()
        .map(|r| {
            let name: String = r.get("name");
            Arc::new(name)
        })
        .collect();

    Ok(schema)
}
