use crate::config::DatabaseConfig;
use crate::error::Error;
use crate::proxy::encrypt_config::{EncryptConfig, EncryptConfigManager};
use crate::proxy::{AGGREGATE_QUERY, SCHEMA_QUERY};
use crate::{connect, log::SCHEMA};
use arc_swap::ArcSwap;
use cipherstash_client::{
    eql::Identifier,
    schema::column::{Index, IndexType},
};
use eql_mapper::{self, EqlTrait, EqlTraits};
use eql_mapper::{Column, Schema, Table};
use sqltk::parser::ast::Ident;
use std::sync::Arc;
use std::time::Duration;
use tokio::{task::JoinHandle, time};
use tracing::{debug, info, warn};

#[derive(Clone, Debug)]
pub struct SchemaManager {
    config: DatabaseConfig,
    encrypt_config_manager: EncryptConfigManager,
    schema: Arc<ArcSwap<Schema>>,
    _reload_handle: Arc<JoinHandle<()>>,
}

impl SchemaManager {
    pub async fn init(
        config: &DatabaseConfig,
        encrypt_config_manager: EncryptConfigManager,
    ) -> Result<Self, Error> {
        let config = config.clone();
        init_reloader(config, encrypt_config_manager).await
    }

    pub fn load(&self) -> Arc<Schema> {
        self.schema.load().clone()
    }

    pub async fn reload(&self) {
        let encrypt_config = self.encrypt_config_manager.load();
        match load_schema_with_retry(&self.config, &encrypt_config).await {
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

async fn init_reloader(
    config: DatabaseConfig,
    encrypt_config_manager: EncryptConfigManager,
) -> Result<SchemaManager, Error> {
    // Skip retries on startup as the likely failure mode is configuration
    let initial_encrypt_config = encrypt_config_manager.load();
    let schema = load_schema(&config, &initial_encrypt_config).await?;
    info!(msg = "Loaded database schema");

    let schema = Arc::new(ArcSwap::new(Arc::new(schema)));

    let config_ref = config.clone();
    let schema_ref = schema.clone();
    let encrypt_config_ref = encrypt_config_manager.clone();

    let reload_handle = tokio::spawn(async move {
        let reload_interval = tokio::time::Duration::from_secs(config_ref.config_reload_interval);

        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + reload_interval,
            reload_interval,
        );

        loop {
            interval.tick().await;

            let encrypt_config = encrypt_config_ref.load();
            match load_schema_with_retry(&config_ref, &encrypt_config).await {
                Ok(reloaded) => {
                    schema_ref.swap(Arc::new(reloaded));
                }
                Err(err) => {
                    warn!(
                        msg = "Error loading database schema",
                        error = err.to_string()
                    );
                }
            }
        }
    });

    Ok(SchemaManager {
        config,
        encrypt_config_manager,
        schema,
        _reload_handle: Arc::new(reload_handle),
    })
}

/// Fetch the dataset and retry on any error
///
/// When databases and the proxy start up at the same time they might not be ready to accept connections before the
/// proxy tries to query the schema. To give the proxy the best chance of initialising correctly this method will
/// retry the query a few times before passing on the error.
async fn load_schema_with_retry(
    config: &DatabaseConfig,
    encrypt_config: &EncryptConfig,
) -> Result<Schema, Error> {
    let mut retry_count = 0;
    let max_retry_count = 10;
    let max_backoff = Duration::from_secs(2);

    loop {
        match load_schema(config, encrypt_config).await {
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

pub async fn load_schema(
    config: &DatabaseConfig,
    encrypt_config: &EncryptConfig,
) -> Result<Schema, Error> {
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
                    let identifier = Identifier::new(&table_name, col);
                    let eql_traits = encrypt_config
                        .get_column_config(&identifier)
                        .map(|config| eql_traits_from_indexes(&config.indexes))
                        .unwrap_or_default();
                    debug!(
                        target: SCHEMA,
                        msg = "eql_v2_encrypted column",
                        table = table_name,
                        column = col,
                        traits = %eql_traits,
                    );
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

/// Translate the configured indexes for a column into the `EqlTraits` that
/// describe which SQL operations the eql-mapper should permit on it.
///
/// Mapping:
///   - `unique`      → `Eq`
///   - `ore` / `ope` → `Ord` (implies `Eq`)
///   - `match`       → `TokenMatch`
///   - `ste_vec`     → `JsonLike` (implies `Ord` + `Eq`) and `Contain`
fn eql_traits_from_indexes(indexes: &[Index]) -> EqlTraits {
    indexes
        .iter()
        .flat_map(|index| match &index.index_type {
            IndexType::Ore | IndexType::Ope => &[EqlTrait::Ord][..],
            IndexType::Match { .. } => &[EqlTrait::TokenMatch][..],
            IndexType::Unique { .. } => &[EqlTrait::Eq][..],
            IndexType::SteVec { .. } => &[EqlTrait::JsonLike, EqlTrait::Contain][..],
        })
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cipherstash_client::schema::column::{Index, Tokenizer};

    #[test]
    fn no_indexes_yields_no_traits() {
        let traits = eql_traits_from_indexes(&[]);
        assert_eq!(traits, EqlTraits::none());
    }

    #[test]
    fn unique_index_yields_eq() {
        let traits = eql_traits_from_indexes(&[Index::new_unique()]);
        assert_eq!(traits, EqlTraits::from(EqlTrait::Eq));
    }

    #[test]
    fn ore_index_yields_ord_and_eq() {
        let traits = eql_traits_from_indexes(&[Index::new_ore()]);
        assert!(traits.ord);
        assert!(traits.eq, "Ord implies Eq");
        assert!(!traits.token_match);
        assert!(!traits.json_like);
        assert!(!traits.contain);
    }

    #[test]
    fn ope_index_yields_ord_and_eq() {
        let traits = eql_traits_from_indexes(&[Index::new_ope()]);
        assert!(traits.ord);
        assert!(traits.eq, "Ord implies Eq");
        assert!(!traits.token_match);
        assert!(!traits.json_like);
        assert!(!traits.contain);
    }

    #[test]
    fn match_index_yields_token_match_only() {
        let traits = eql_traits_from_indexes(&[Index::new(IndexType::Match {
            tokenizer: Tokenizer::Standard,
            token_filters: vec![],
            k: 6,
            m: 2048,
            include_original: false,
        })]);
        assert!(traits.token_match);
        assert!(!traits.eq);
        assert!(!traits.ord);
    }

    #[test]
    fn ste_vec_index_yields_json_like_and_contain() {
        let traits = eql_traits_from_indexes(&[Index::new(IndexType::SteVec {
            prefix: "doc".into(),
            term_filters: vec![],
            array_index_mode: Default::default(),
        })]);
        assert!(traits.json_like);
        assert!(traits.contain);
        assert!(traits.ord, "JsonLike implies Ord");
        assert!(traits.eq, "JsonLike implies Eq");
        assert!(!traits.token_match);
    }

    #[test]
    fn multiple_indexes_unioned() {
        let traits = eql_traits_from_indexes(&[
            Index::new_ore(),
            Index::new_unique(),
            Index::new(IndexType::Match {
                tokenizer: Tokenizer::Standard,
                token_filters: vec![],
                k: 6,
                m: 2048,
                include_original: false,
            }),
        ]);
        assert!(traits.eq);
        assert!(traits.ord);
        assert!(traits.token_match);
        assert!(!traits.json_like);
        assert!(!traits.contain);
    }
}
