mod common;

use cipherstash_proxy::{config::SchemaManager, trace};
use common::database_config;
use tracing::info;

#[tokio::test]
async fn load_schema() {
    trace();

    let config = database_config();
    let manager = SchemaManager::init(&config).await.unwrap();

    // info!("SchemaManager: {:?}", manager);

    let tables = manager.schema.tables.clone();
    info!("tables: {:?}", tables);
}
