#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, id, PROXY};
    use cipherstash_proxy::{config::LogConfig, log};

    #[tokio::test]
    async fn schema_change_reloads_schema() {
        let client = connect_with_tls(PROXY).await;

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

        assert!(rows.is_empty());
    }
}
