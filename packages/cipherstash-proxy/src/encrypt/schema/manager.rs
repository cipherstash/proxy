use crate::config::DatabaseConfig;
use crate::encrypt::{AGGREGATE_QUERY, SCHEMA_QUERY};
use crate::error::Error;
use crate::{connect, log::SCHEMA};
use arc_swap::ArcSwap;
use eql_mapper::{self, EqlTraits};
use eql_mapper::{Column, Schema, Table};
use sqltk::parser::ast::Ident;
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
                debug!(target: SCHEMA, msg = "Reloaded database schema");
                self.schema.swap(Arc::new(reloaded));
            }
            Err(err) => {
                warn!(
                    msg = "Error reloading database schema",
                    error = err.to_string()
                );
            }
        };
    }
}

async fn init_reloader(config: DatabaseConfig) -> Result<SchemaManager, Error> {
    // Skip retries on startup as the likely failure mode is configuration
    let schema = load_schema(&config).await?;
    info!(msg = "Loaded database schema");

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
                    schema_ref.swap(Arc::new(reloaded));
                }
                Err(err) => {
                    warn!(
                        msg = "Error loading Encrypt configuration",
                        error = err.to_string()
                    );
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
        warn!(msg = "Database schema contains no tables");
        return Ok(schema);
    };

    for table in tables {
        let table_name: String = table.get("table_name");
        let columns: Vec<String> = table.get("columns");
        let column_type_names: Vec<Option<String>> = table.get("column_type_names");

        let mut table = Table::new(Ident::new(&table_name));

        columns.iter().zip(column_type_names).for_each(|(col, column_type_name)| {
            let ident = Ident::with_quote('"', col);

            let column = match column_type_name.as_deref() {
                Some("eql_v2_encrypted") => {
                    debug!(target: SCHEMA, msg = "eql_v2_encrypted column", table = table_name, column = col);

                    // TODO - map config to the set of implemented traits
                    let eql_traits =  EqlTraits::all();
                    Column::eql(ident, eql_traits)
                }
                _ => Column::native(ident),
            };

            table.add_column(Arc::new(column));
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
