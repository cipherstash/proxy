#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, database_config_with_port, PROXY};
    use cipherstash_proxy::log;

    fn id() -> i64 {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        rng.gen_range(1..=i64::MAX)
    }

    #[tokio::test]
    async fn schema_change_reloads_schema() {
        log::init();

        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

        let id = id();

        let sql = format!(
            "CREATE TABLE table_{id} (
            id bigint,
            PRIMARY KEY(id)
        );"
        );

        let _ = client.execute(&sql, &[]).await.expect("ok");

        let sql = format!("SELECT id FROM table_{id}");
        let rows = client.query(&sql, &[]).await.expect("ok");

        assert!(rows.len() == 0);
    }
}
