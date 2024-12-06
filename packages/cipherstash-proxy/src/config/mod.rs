mod config_manager;
mod encrypt_config;
mod schema_manager;
mod tandem;

pub use config_manager::EncryptConfigManager;
pub use encrypt_config::EncryptConfig;
pub use schema_manager::SchemaManager;

pub use tandem::{DatabaseConfig, ServerConfig, TandemConfig, TlsConfig};

pub const CS_PREFIX: &str = "CS";
pub const DEFAULT_CONFIG_FILE_PATH: &str = "cipherstash-proxy.toml";

const ENCRYPT_DATASET_CONFIG_QUERY: &'static str = include_str!("./sql/select_config.sql");

const SCHEMA_QUERY: &'static str = include_str!("./sql/select_table_schemas.sql");

const AGGREGATE_QUERY: &'static str = include_str!("./sql/select_aggregates.sql");
