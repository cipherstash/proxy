#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, random_id, PROXY};

    #[tokio::test]
    async fn schema_change_reloads_schema() {
        let client = connect_with_tls(PROXY).await;

        let id = random_id();

        let sql = format!(
            "CREATE TABLE table_{id} (
            id bigint,
            PRIMARY KEY(id)
        );"
        );

        let _ = client.execute(&sql, &[]).await.unwrap();

        let sql = format!("SELECT id FROM table_{id}");
        let rows = client.query(&sql, &[]).await.unwrap();

        assert!(rows.is_empty());
    }
}
