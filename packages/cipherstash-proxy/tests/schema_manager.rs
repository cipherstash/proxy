mod common;

use cipherstash_proxy::{config::SchemaManager, log};
use common::database_config;
use sqlparser::ast::Ident;

#[tokio::test]
async fn integration_load_schema() {
    log::init();

    let config = database_config();
    let manager = SchemaManager::init(&config).await.unwrap();

    let schema = manager.load();

    assert!(!schema.tables.is_empty());

    assert!(schema
        .resolve_table_column(
            &Ident::with_quote('"', "pg_database"),
            &Ident::with_quote('"', "datname")
        )
        .is_ok());
}
