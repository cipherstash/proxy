mod common;

use cipherstash_proxy::{config::SchemaManager, log};
use common::database_config;

#[tokio::test]
async fn integration_load_schema() {
    log::init();

    let config = database_config();
    let manager = SchemaManager::init(&config).await.unwrap();

    let schema = manager.load();

    // info!("schema.tables: {:?}", schema.tables);

    assert!(!schema.tables.is_empty());
}
