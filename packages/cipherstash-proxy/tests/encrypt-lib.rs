#![allow(dead_code)]

use cipherstash_proxy::{log, Encrypt, TandemConfig};
use tracing::info;

mod common;

///
/// Sanity test to check if the database connection is working with TLS
///
#[tokio::test]
async fn integration_encrypt_as_a_lib() {
    log::init();

    let config_file = "tests/config/cipherstash-proxy.toml";

    let config = TandemConfig::load(config_file).unwrap();

    let encrypt = Encrypt::init(config).await.unwrap();
    info!("Connected to CipherStash Encrypt");
    info!("Connected to database: {}", encrypt.config.database);

    // call encrypt and decrypt here
    // encrypt.encrypt(plaintexts).await.unwrap();
    // encrypt.decrypt(ciphertexts).await.unwrap();
}
