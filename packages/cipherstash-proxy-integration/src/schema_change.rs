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

    #[tokio::test]
    async fn disable_mapping_disables_schema_reload() {
        let client = connect_with_tls(PROXY).await;

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
        client.query(sql, &[]).await.unwrap();

        let id = random_id();

        let sql = format!(
            "CREATE TABLE table_{id} (
            id bigint,
            PRIMARY KEY(id)
        );"
        );

        let _ = client.execute(&sql, &[]).await.unwrap();

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false";
        client.query(sql, &[]).await.unwrap();

        let sql = format!("SELECT id FROM table_{id}");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_err());
    }
}
