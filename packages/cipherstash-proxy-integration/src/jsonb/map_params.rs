#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, id, reset_schema, trace, PROXY};

    #[tokio::test]
    async fn with_number() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_jsonb = serde_json::json!({"key": 42});

        let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_jsonb]).await.unwrap();

        let sql = "SELECT id, encrypted_jsonb FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result: serde_json::Value = row.get("encrypted_jsonb");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_jsonb, result);
        }
    }

    #[tokio::test]
    async fn with_array() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_jsonb = serde_json::json!({"a": [1, 2, 4, 42]});

        let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_jsonb]).await.unwrap();

        let sql = "SELECT id, encrypted_jsonb FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result: serde_json::Value = row.get("encrypted_jsonb");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_jsonb, result);
        }
    }
}
