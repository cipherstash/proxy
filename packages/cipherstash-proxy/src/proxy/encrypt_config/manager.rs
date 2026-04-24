use crate::{
    config::DatabaseConfig,
    connect,
    error::{ConfigError, Error},
    log::ENCRYPT_CONFIG,
    proxy::ENCRYPT_CONFIG_QUERY,
};
use arc_swap::ArcSwap;
use cipherstash_client::eql;
use cipherstash_client::schema::ColumnConfig;
use cipherstash_config::CanonicalEncryptionConfig;
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
                Error::Config(ConfigError::InvalidEncryptionConfig(ref inner)) => {
                    error!(
                        msg = "Invalid Encrypt configuration in database",
                        error = inner.to_string()
                    );
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
            let canonical: CanonicalEncryptionConfig = serde_json::from_value(json_value)?;
            let encrypt_config = EncryptConfig::new_from_config(canonical_to_map(canonical)?);

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

fn canonical_to_map(canonical: CanonicalEncryptionConfig) -> Result<EncryptConfigMap, ConfigError> {
    Ok(canonical
        .into_config_map()?
        .into_iter()
        .map(|(id, col)| (eql::Identifier::new(id.table, id.column), col))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cipherstash_client::eql::Identifier;
    use cipherstash_config::column::{ArrayIndexMode, IndexType, TokenFilter, Tokenizer};
    use cipherstash_config::ColumnType;
    use serde_json::json;

    fn parse(json: serde_json::Value) -> EncryptConfigMap {
        let config: CanonicalEncryptionConfig = serde_json::from_value(json).unwrap();
        canonical_to_map(config).unwrap()
    }

    #[test]
    fn column_with_empty_options_gets_defaults() {
        let json = json!({
            "v": 1,
            "tables": { "users": { "email": {} } }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "email"))
            .unwrap();

        assert_eq!(column.cast_type, ColumnType::Text);
        assert!(column.indexes.is_empty());
    }

    #[test]
    fn can_parse_column_with_cast_as() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": { "favourite_int": { "cast_as": "int" } }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "favourite_int"))
            .unwrap();

        assert_eq!(column.cast_type, ColumnType::Int);
        assert_eq!(column.name, "favourite_int");
        assert!(column.indexes.is_empty());
    }

    #[test]
    fn cast_as_real_maps_to_float() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": { "rating": { "cast_as": "real" } }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "rating"))
            .unwrap();

        assert_eq!(column.cast_type, ColumnType::Float);
    }

    #[test]
    fn cast_as_double_maps_to_float() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": { "rating": { "cast_as": "double" } }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "rating"))
            .unwrap();

        assert_eq!(column.cast_type, ColumnType::Float);
    }

    #[test]
    fn can_parse_empty_indexes() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": { "email": { "indexes": {} } }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "email"))
            .unwrap();

        assert!(column.indexes.is_empty());
    }

    #[test]
    fn can_parse_ore_index() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": { "email": { "indexes": { "ore": {} } } }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "email"))
            .unwrap();

        assert_eq!(column.indexes[0].index_type, IndexType::Ore);
    }

    #[test]
    fn can_parse_unique_index_with_defaults() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": { "email": { "indexes": { "unique": {} } } }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "email"))
            .unwrap();

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::Unique {
                token_filters: vec![]
            }
        );
    }

    #[test]
    fn can_parse_unique_index_with_token_filter() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "indexes": {
                            "unique": {
                                "token_filters": [{ "kind": "downcase" }]
                            }
                        }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "email"))
            .unwrap();

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::Unique {
                token_filters: vec![TokenFilter::Downcase]
            }
        );
    }

    #[test]
    fn can_parse_match_index_with_defaults() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": { "email": { "indexes": { "match": {} } } }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "email"))
            .unwrap();

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::Match {
                tokenizer: Tokenizer::Standard,
                token_filters: vec![],
                k: 6,
                m: 2048,
                include_original: false,
            }
        );
    }

    #[test]
    fn can_parse_match_index_with_all_opts_set() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "indexes": {
                            "match": {
                                "tokenizer": { "kind": "ngram", "token_length": 3 },
                                "token_filters": [{ "kind": "downcase" }],
                                "k": 8,
                                "m": 1024,
                                "include_original": true
                            }
                        }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "email"))
            .unwrap();

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::Match {
                tokenizer: Tokenizer::Ngram { token_length: 3 },
                token_filters: vec![TokenFilter::Downcase],
                k: 8,
                m: 1024,
                include_original: true,
            }
        );
    }

    #[test]
    fn can_parse_ste_vec_index() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "event_data": {
                        "cast_as": "jsonb",
                        "indexes": { "ste_vec": { "prefix": "event-data" } }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get(&Identifier::new("users", "event_data"))
            .unwrap();

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::SteVec {
                prefix: "event-data".into(),
                term_filters: vec![],
                array_index_mode: ArrayIndexMode::ALL,
            },
        );
    }

    #[test]
    fn config_map_preserves_table_and_column_names() {
        let json = json!({
            "v": 1,
            "tables": {
                "my_schema.users": {
                    "email_address": {
                        "cast_as": "text",
                        "indexes": { "unique": {} }
                    }
                }
            }
        });

        let config = parse(json);
        let column = config
            .get(&Identifier::new("my_schema.users", "email_address"))
            .unwrap();
        assert_eq!(column.name, "email_address");
        assert_eq!(column.cast_type, ColumnType::Text);
    }

    #[test]
    fn config_map_handles_multiple_tables() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": { "email": { "cast_as": "text" } },
                "orders": { "total": { "cast_as": "int" } }
            }
        });

        let config = parse(json);

        assert_eq!(config.len(), 2);
        assert_eq!(
            config
                .get(&Identifier::new("users", "email"))
                .unwrap()
                .cast_type,
            ColumnType::Text
        );
        assert_eq!(
            config
                .get(&Identifier::new("orders", "total"))
                .unwrap()
                .cast_type,
            ColumnType::Int
        );
    }

    #[test]
    fn invalid_config_returns_error() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "cast_as": "text",
                        "indexes": { "ste_vec": { "prefix": "test" } }
                    }
                }
            }
        });

        let config: CanonicalEncryptionConfig = serde_json::from_value(json).unwrap();
        assert!(canonical_to_map(config).is_err());
    }

    #[test]
    fn real_eql_config_produces_correct_encrypt_config() {
        let json = json!({
            "v": 1,
            "tables": {
                "encrypted": {
                    "encrypted_text": {
                        "cast_as": "text",
                        "indexes": { "unique": {}, "match": {}, "ore": {} }
                    },
                    "encrypted_bool": {
                        "cast_as": "boolean",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_int2": {
                        "cast_as": "small_int",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_int4": {
                        "cast_as": "int",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_int8": {
                        "cast_as": "big_int",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_float8": {
                        "cast_as": "double",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_date": {
                        "cast_as": "date",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_jsonb": {
                        "cast_as": "jsonb",
                        "indexes": {
                            "ste_vec": { "prefix": "encrypted/encrypted_jsonb" }
                        }
                    },
                    "encrypted_jsonb_filtered": {
                        "cast_as": "jsonb",
                        "indexes": {
                            "ste_vec": {
                                "prefix": "encrypted/encrypted_jsonb_filtered",
                                "term_filters": [{ "kind": "downcase" }]
                            }
                        }
                    }
                }
            }
        });

        let config = parse(json);

        assert_eq!(config.len(), 9);

        assert_eq!(
            config
                .get(&Identifier::new("encrypted", "encrypted_float8"))
                .unwrap()
                .cast_type,
            ColumnType::Float
        );
        assert_eq!(
            config
                .get(&Identifier::new("encrypted", "encrypted_jsonb"))
                .unwrap()
                .cast_type,
            ColumnType::Json
        );
        assert_eq!(
            config
                .get(&Identifier::new("encrypted", "encrypted_text"))
                .unwrap()
                .indexes
                .len(),
            3
        );
        assert_eq!(
            config
                .get(&Identifier::new("encrypted", "encrypted_bool"))
                .unwrap()
                .indexes
                .len(),
            2
        );
        assert_eq!(
            config
                .get(&Identifier::new("encrypted", "encrypted_jsonb_filtered"))
                .unwrap()
                .indexes
                .len(),
            1
        );
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let json = json!({
            "v": 1,
            "tables": "not a map"
        });

        let result = serde_json::from_value::<CanonicalEncryptionConfig>(json);
        assert!(result.is_err());
    }
}
