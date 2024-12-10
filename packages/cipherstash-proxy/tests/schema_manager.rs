mod common;

use cipherstash_proxy::{config::SchemaManager, trace};
use common::database_config;
use tracing::info;

// #[tokio::test]
async fn load_schema() {
    trace();

    let config = database_config();
    let manager = SchemaManager::init(&config).await.unwrap();

    let schema = manager.load();

    // info!("schema.tables: {:?}", schema.tables);

    assert!(!schema.tables.is_empty());
}
