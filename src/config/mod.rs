mod dataset;
mod dataset_manager;
mod tandem;

pub use dataset::JsonDatasetConfig;
pub use dataset_manager::DatasetManager;
pub use tandem::{TandemConfig, TlsConfig};

pub const CS_PREFIX: &str = "CS";
pub const DEFAULT_CONFIG_FILE_PATH: &str = "cipherstash-proxy.toml";

const ENCRYPT_DATASET_CONFIG_QUERY: &'static str = include_str!("./sql/select_config.sql");
