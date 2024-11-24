use my_little_proxy::{trace, TandemConfig};
use tracing::info;

#[tokio::test]
async fn test_tls() {
    trace();
}
