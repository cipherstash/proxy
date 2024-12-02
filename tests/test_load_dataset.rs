use cipherstash_proxy::{trace, TandemConfig};
use tracing::info;

#[tokio::test]
async fn test_load_dataset() {
    trace();

    let config = TandemConfig::load("tests/cipherstash-proxy.toml").unwrap();
    // config.connect.reload_interval = 10;
    info!("config: {:?}", config);

    // let dataset = load_dataset_config(&config).await.unwrap();

    // info!("dataset: {:?}", dataset);
}
