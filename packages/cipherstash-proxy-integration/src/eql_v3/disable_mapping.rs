//! EQL v3 variant of `disable_mapping.rs`.
//!
//! With `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true` the proxy passes raw
//! column values through. In v2 those are `eql_v2_encrypted` composites; in v3
//! each column is a jsonb-backed `eql_v3.*` domain, so the raw value is the v3
//! envelope `{v: 3, i: {t, c}, c, <terms>}` with no `k` discriminator.

#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, execute_query, query_with_client, random_id,
        simple_query_with_client, trace, PROXY,
    };
    use serde_json::{json, Value};

    ///
    /// Tests mapping is disabled when the `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` command is used
    /// and the raw value is an EQL v3 envelope.
    ///
    #[tokio::test]
    #[ignore = "blocked on eql-mapper v3"]
    async fn select_with_set_unsafe_disable_mapping_returns_v3_envelope() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_text = "hello".to_string();
        let expected = vec![encrypted_text.to_owned()];

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        execute_query(sql, &[&id, &encrypted_text]).await;

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
        client.query(sql, &[]).await.unwrap();

        // Data should not be decrypted: the raw jsonb envelope comes back.
        // Scoped by id so the test stays parallel-safe on the shared table.
        let select_sql = format!("SELECT encrypted_text FROM encrypted WHERE id = {id}");
        let rows = simple_query_with_client::<String>(&select_sql, &client).await;

        assert_eq!(rows.len(), 1);

        for s in rows {
            let envelope: Value = serde_json::from_str(&s).unwrap();
            // v3 envelope: version 3, identifier, ciphertext - and no v2 `k` discriminator
            assert_eq!(envelope["v"], json!(3));
            assert_eq!(envelope["i"]["c"], json!("encrypted_text"));
            assert!(envelope.get("c").is_some());
            assert!(envelope.get("k").is_none());
        }

        // ---------------------
        // Turn mapping back on and regular services resume

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false";
        client.query(sql, &[]).await.unwrap();

        let actual = query_with_client::<String>(&select_sql, &client).await;
        assert_eq!(expected, actual);
    }
}
