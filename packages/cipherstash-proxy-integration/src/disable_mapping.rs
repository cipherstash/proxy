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
    #[postgres(name = "eql_v2_encrypted")]
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

        // This error is actually a `WrongType` error from the tokio client as encrypted_text is actually eql_v2_encrypted
        assert!(result.is_err());

        // Force the eql_v2_encrypted type
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

        // Data should not be decrypted
        let select_sql = "SELECT encrypted_text FROM encrypted";
        let rows = query_with_client::<EqlEncrypted>(select_sql, &client).await;

        assert_eq!(rows.len(), 1);

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

    ///
    /// Tests that a connection pool's reset turns mapping back on, so that a disabled mapping
    /// is not carried over to the next client of a pooled connection.
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn pool_reset_turns_mapping_back_on() {
        trace();

        clear().await;

        let id = random_id();
        let encrypted_text = "hello".to_string();
        let expected = vec![encrypted_text.to_owned()];

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        execute_query(sql, &[&id, &encrypted_text]).await;

        let select_sql = "SELECT encrypted_text FROM encrypted";
        let client = connect_with_tls(PROXY).await;

        for reset in [
            "DISCARD ALL",
            "RESET ALL",
            "RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING",
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT",
        ] {
            // Simple query
            client
                .simple_query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
                .await
                .unwrap();
            client.simple_query(reset).await.unwrap();

            let actual = query_with_client::<String>(select_sql, &client).await;
            assert_eq!(expected, actual, "{reset}");

            // Extended query
            client
                .execute("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", &[])
                .await
                .unwrap();
            client.execute(reset, &[]).await.unwrap();

            let actual = simple_query_with_client::<String>(select_sql, &client).await;
            assert_eq!(expected, actual, "{reset}");
        }
    }

    ///
    /// Tests that a change to `CIPHERSTASH.UNSAFE_DISABLE_MAPPING` in a rolled-back transaction is
    /// undone, as PostgreSQL undoes it. Runs with `default_keyset_id` configured.
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn rolled_back_mapping_changes_are_undone() {
        trace();

        clear().await;

        let id = random_id();
        let encrypted_text = "hello".to_string();
        let expected = vec![encrypted_text.to_owned()];

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        execute_query(sql, &[&id, &encrypted_text]).await;

        let select_sql = "SELECT encrypted_text FROM encrypted";
        let client = connect_with_tls(PROXY).await;

        // A rolled-back SET leaves mapping on
        client.simple_query("BEGIN").await.unwrap();
        client
            .simple_query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
            .await
            .unwrap();
        client.simple_query("ROLLBACK").await.unwrap();

        let actual = query_with_client::<String>(select_sql, &client).await;
        assert_eq!(expected, actual);

        client.execute("BEGIN", &[]).await.unwrap();
        client
            .execute("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", &[])
            .await
            .unwrap();
        client.execute("ROLLBACK", &[]).await.unwrap();

        let actual = simple_query_with_client::<String>(select_sql, &client).await;
        assert_eq!(expected, actual);

        // A rolled-back reset leaves mapping off
        for reset in [
            "RESET ALL",
            "RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING",
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT",
        ] {
            // Simple query
            client
                .simple_query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
                .await
                .unwrap();
            client.simple_query("BEGIN").await.unwrap();
            client.simple_query(reset).await.unwrap();
            client.simple_query("ROLLBACK").await.unwrap();

            // Undecrypted, so the raw EQL payload still carries the column identifier
            let rows = query_with_client::<EqlEncrypted>(select_sql, &client).await;
            assert_eq!(rows.len(), 1, "{reset}");
            assert_eq!(rows[0].data["i"]["c"], "encrypted_text", "{reset}");

            // Extended query
            client.execute("BEGIN", &[]).await.unwrap();
            client.execute(reset, &[]).await.unwrap();
            client.execute("ROLLBACK", &[]).await.unwrap();

            let rows = simple_query_with_client::<String>(select_sql, &client).await;
            assert_eq!(rows.len(), 1, "{reset}");
            assert!(rows[0].contains("encrypted_text"), "{reset}");

            client
                .simple_query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false")
                .await
                .unwrap();
        }
    }
}
