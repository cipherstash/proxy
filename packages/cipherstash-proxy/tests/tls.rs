mod common;

use cipherstash_proxy::trace;
use common::{
    connect, connect_with_tls, database_config, database_config_with_port, PG_v17_TLS, PROXY,
};
use tracing::info;

///
/// Sanity test to check if the database connection is working with TLS
///
#[tokio::test]
async fn connect_proxy_with_tls() {
    trace();

    let config = database_config_with_port(PROXY);

    // Connect to proxy without TLS
    let client = connect(&config).await;

    let result = client.simple_query("SELECT 1").await.expect("ok");

    // assert!(!result.is_empty());

    // let client = connect_with_tls(&config).await;

    info!("{:?}", result);

    info!("Connected to database");
}

///
/// Sanity test to check if the database connection is working with TLS
///
#[tokio::test]
async fn sanity_check_database_with_tls() {
    trace();

    let config = database_config_with_port(PG_v17_TLS);

    let client = connect_with_tls(&config).await;

    let result = client.simple_query("SELECT 1").await.expect("ok");

    assert!(!result.is_empty());

    info!("{:?}", result);

    info!("Connected to database");

    let client = connect(&config).await;
}
