#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, execute_query, query_with_client, random_id,
        simple_query_with_client, trace, PROXY,
    };
    use serde::Deserialize;
    use serde_json::Value;
    use tokio_postgres::types::{FromSql, ToSql};

    #[derive(Clone, Debug, ToSql, FromSql, PartialEq, Deserialize)]
    #[postgres(name = "eql_v3_text_search")]
    pub struct EqlEncrypted {
        pub data: Value,
    }

    ///
    /// Tests mapping is disabled when the `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`` command is used
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn insert_with_set_unsafe_disable_mapping() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_text = "hello".to_string();

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
        client.query(sql, &[]).await.unwrap();

        let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        let result = client.query(insert_sql, &[&id, &encrypted_text]).await;

        // This error is actually a `WrongType` error from the tokio client as encrypted_text is actually eql_v3_text_search
        assert!(result.is_err());

        // Force the eql_v3_text_search type
        let encrypted = EqlEncrypted {
            data: Value::from(encrypted_text.to_owned()),
        };

        let result = client.query(insert_sql, &[&id, &encrypted]).await;

        assert!(result.is_err());

        // ---------------------
        // Simple query with same client

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')");
        let result = client.simple_query(&sql).await;
        assert!(result.is_err());

        // ---------------------
        // Turn mapping back on and regular services resume
        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false";
        client.query(sql, &[]).await.unwrap();

        let result = client.query(insert_sql, &[&id, &encrypted_text]).await;
        assert!(result.is_ok());
    }

    ///
    /// Tests mapping is disabled when the `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`` command is used
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn select_with_set_unsafe_disable_mapping() {
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

        // Data should not be decrypted.
        //
        // Read it as `Value`, not as `EqlEncrypted`: a v3 encrypted column is a
        // DOMAIN over `jsonb`, and PostgreSQL reports a domain's *base* type in
        // RowDescription. The client therefore sees `jsonb`, and a struct whose
        // `FromSql` is pinned to the domain name is rejected outright. (Under v2
        // this worked because `eql_v2_encrypted` was a real composite type with
        // its own OID.)
        let select_sql = "SELECT encrypted_text FROM encrypted";
        let rows = query_with_client::<Value>(select_sql, &client).await;

        assert_eq!(rows.len(), 1);
        // Undecrypted, so the raw EQL payload still carries the column identifier.
        assert_eq!(rows[0]["i"]["c"], "encrypted_text");

        // Simple query using same client
        let rows = simple_query_with_client::<String>(select_sql, &client).await;
        assert_eq!(rows.len(), 1);
        // String is the jsonb and should include identifier
        for s in rows {
            assert!(s.contains("encrypted_text"))
        }

        // --------------
        // Turn mapping back on and regular services resume

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false";
        client.query(sql, &[]).await.unwrap();

        let actual = query_with_client::<String>(select_sql, &client).await;
        assert_eq!(actual.len(), 1);
        assert_eq!(expected, actual);

        let rows = simple_query_with_client::<String>(select_sql, &client).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(expected, actual);
    }

    ///
    /// Tests mapping is disabled when the `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`` command is used
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn select_with_set_unsafe_disable_mapping_bound_to_client() {
        trace();

        clear().await;

        let id = random_id();

        let encrypted_text = "hello".to_string();
        let expected = vec![encrypted_text.to_owned()];

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        execute_query(sql, &[&id, &encrypted_text]).await;

        let client = connect_with_tls(PROXY).await;
        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
        client.query(sql, &[]).await.unwrap();

        let select_sql = "SELECT encrypted_text FROM encrypted";

        // Mapping is NOT disabled for these queries
        for _ in 1..5 {
            let client = connect_with_tls(PROXY).await;

            let actual = query_with_client::<String>(select_sql, &client).await;

            assert_eq!(actual.len(), 1);
            assert_eq!(expected, actual);

            let actual = simple_query_with_client::<String>(select_sql, &client).await;
            assert_eq!(actual.len(), 1);
            assert_eq!(expected, actual);
        }
    }
}
