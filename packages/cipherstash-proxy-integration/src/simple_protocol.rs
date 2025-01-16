#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, database_config_with_port, id, PROXY};
    use cipherstash_proxy::log;

    #[tokio::test]
    async fn simple_text() {
        log::init();

        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')");
        client.simple_query(&sql).await.expect("ok");

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.expect("ok");

        assert!(rows.len() == 1);

        for row in rows {
            // let result: String = row.get("encrypted_text");
            // assert_eq!(encrypted_text, result);
        }
    }
}
