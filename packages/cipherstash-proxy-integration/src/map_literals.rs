#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, id, PROXY};

    #[tokio::test]
    async fn map_literal() {
        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')");
        client.query(&sql, &[]).await.expect("INSERT query failed");

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client.query(&sql, &[]).await.expect("SELECT query failed");

        let result: String = rows[0].get("encrypted_text");
        assert_eq!(encrypted_text, result);
    }

    #[tokio::test]
    async fn map_literal_with_param() {
        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";
        let int2: i16 = 1;

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text, encrypted_bool, encrypted_int2) VALUES ({id}, '{encrypted_text}', $1, $2)");
        client
            .query(&sql, &[&true, &int2])
            .await
            .expect("INSERT query failed");

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client.query(&sql, &[]).await.expect("SELECT query failed");

        println!("encrypted: {:?}", rows[0])
    }

    #[tokio::test]
    async fn map_jsonb() {
        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_jsonb = serde_json::json!({"key": "value"});

        let sql = format!(
            "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, '{encrypted_jsonb}')",
        );

        client.query(&sql, &[&id]).await.expect("ok");

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
