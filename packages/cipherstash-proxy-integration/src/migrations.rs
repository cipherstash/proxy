#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, database_config_with_port, PROXY};
    use cipherstash_proxy::log;

    fn _id() -> i64 {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        rng.gen_range(1..=i64::MAX)
    }

    // #[tokio::test]
    async fn unknown_table() {
        log::init();

        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

        let sql = "SELECT version FROM schema_migrations";
        let rows = client.query(sql, &[]).await.expect("ok");

        assert!(rows.len() == 1);

        // for row in rows {
        //     let result: String = row.get("encrypted_text");
        //     assert_eq!(encrypted_text, result);
        // }
    }
}
