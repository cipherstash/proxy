use my_little_proxy::{load_dataset, trace, TandemConfig};
use tracing::info;

#[tokio::test]
async fn test_load_dataset() {
    trace();

    let config = TandemConfig::load("tests/cipherstash-proxy.toml").unwrap();
    info!("config: {:?}", config);

    let dataset = load_dataset(&config).await.unwrap();
}
