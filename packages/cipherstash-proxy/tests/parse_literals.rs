mod common;

use cipherstash_proxy::log;
use common::{connect, database_config_with_port, PROXY};
use tracing::info;

///
/// Sanity test to check if the database connection is working with TLS
///
/// #[tokio::test]
async fn _parse_with_tls() {
    log::init();

    let config = database_config_with_port(PROXY);
    let client = connect(&config).await;

    let _result = client
        .simple_query("INSERT INTO users (email) VALUES ('toby@cipherstash.com')")
        .await
        .expect("ok");

    let result = client
        .simple_query("SELECT * FROM users")
        .await
        .expect("ok");

    info!("{:?}", result);
}
