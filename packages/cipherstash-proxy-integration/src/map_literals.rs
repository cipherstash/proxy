#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, query_direct_by, random_id, trace, PROXY};

    #[tokio::test]
    async fn map_literal() {
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
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
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
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

    /// Verify JSONB literal insertion and retrieval without explicit type casts.
    ///
    /// JSONB literals in INSERT and SELECT statements work directly with the proxy
    /// without requiring `::jsonb` type annotations. The proxy infers the JSONB type
    /// from the target column and handles encryption/decryption transparently.
    #[tokio::test]
    async fn map_jsonb() {
        trace();
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_jsonb = serde_json::json!({"key": "value"});

        let sql = format!(
            "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, '{encrypted_jsonb}')",
        );

        client.query(&sql, &[&id]).await.unwrap();

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

    /// Sanity check: verify JSONB is actually encrypted in database
    ///
    /// This test catches silent encryption failures where plaintext is stored.
    /// Insert via proxy, query DIRECT from database to verify encryption,
    /// then query via proxy to verify decryption round-trip.
    #[tokio::test]
    async fn jsonb_encryption_sanity_check() {
        trace();
        clear().await;

        let id = random_id();
        let plaintext_json = serde_json::json!({"key": "value"});

        // Insert through proxy (should encrypt)
        let client = connect_with_tls(PROXY).await;
        let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
        client.query(sql, &[&id, &plaintext_json]).await.unwrap();

        // Query DIRECT from database (bypassing proxy, no decryption)
        // The stored value should NOT be readable as the original JSON
        let sql = "SELECT encrypted_jsonb::text FROM encrypted WHERE id = $1";
        let stored: Vec<String> = query_direct_by(sql, &id).await;

        assert_eq!(stored.len(), 1, "Expected exactly one row");
        let stored_text = &stored[0];

        // Verify it's NOT the plaintext JSON (encryption actually happened)
        let plaintext_str = plaintext_json.to_string();
        assert_ne!(
            stored_text, &plaintext_str,
            "ENCRYPTION FAILED: Stored value matches plaintext! Data was not encrypted."
        );

        // Additional verification: the encrypted format should be different structure
        if let Ok(stored_json) = serde_json::from_str::<serde_json::Value>(stored_text) {
            assert_ne!(
                stored_json, plaintext_json,
                "ENCRYPTION FAILED: Stored JSON structure matches plaintext!"
            );
        }

        // Round-trip: query through proxy should decrypt back to original
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();
        assert_eq!(rows.len(), 1, "Expected exactly one row for round-trip");
        let decrypted: serde_json::Value = rows[0].get(0);
        assert_eq!(
            decrypted, plaintext_json,
            "DECRYPTION FAILED: Round-trip value doesn't match original!"
        );
    }

    #[tokio::test]
    async fn map_repeated_literals_different_columns_regression() {
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_int8) VALUES ({id}, {id}) RETURNING id, encrypted_int8");
        let rows = client.query(&sql, &[]).await.unwrap();

        let actual = rows
            .iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect::<Vec<(i64, i64)>>();

        let expected = vec![(id, id)];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_repeated_literals_same_column_regression() {
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({}, 'a'), ({}, 'a') RETURNING encrypted_text", random_id(), random_id());
        let rows = client.query(&sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<&str>>();

        let expected = vec!["a", "a"];

        assert_eq!(actual, expected);
    }
}
