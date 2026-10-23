#[cfg(test)]
mod tests {
    use crate::common::{
        assert_db_error, clear, connect_with_tls, execute_query, query_direct_by,
        query_with_client, random_id, simple_query_with_client, trace, PROXY,
    };
    use cipherstash_client::eql::EqlCiphertextV3;
    use serde::Deserialize;
    use serde_json::Value;
    use tokio_postgres::types::{FromSql, ToSql};
    use uuid::Uuid;

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

        let client = connect_with_tls(*PROXY).await;

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

        let client = connect_with_tls(*PROXY).await;

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

        let client = connect_with_tls(*PROXY).await;
        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
        client.query(sql, &[]).await.unwrap();

        let select_sql = "SELECT encrypted_text FROM encrypted";

        // Mapping is NOT disabled for these queries
        for _ in 1..5 {
            let client = connect_with_tls(*PROXY).await;

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
        let client = connect_with_tls(*PROXY).await;

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
    /// undone, as PostgreSQL undoes it, and that encrypted writes still use the default keyset.
    /// Runs with `default_keyset_id` configured, and asserts it.
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn rolled_back_mapping_changes_are_undone() {
        trace();

        clear().await;

        // Proxy runs with `default_keyset_id` configured, so it refuses to select a keyset
        let other = connect_with_tls(*PROXY).await;
        assert_db_error(
            other
                .simple_query("SET CIPHERSTASH.KEYSET_NAME = 'tenant-1'")
                .await,
            "FATAL",
            "Cannot SET CIPHERSTASH.KEYSET if a default keyset has been configured. For help visit https://github.com/cipherstash/proxy/blob/main/docs/errors.md#encrypt-unexpected-set-keyset",
        );

        let id = random_id();
        let encrypted_text = "hello".to_string();
        let expected = vec![encrypted_text.to_owned()];

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        execute_query(sql, &[&id, &encrypted_text]).await;

        let select_sql = "SELECT encrypted_text FROM encrypted";
        let client = connect_with_tls(*PROXY).await;

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
            let rows = query_with_client::<Value>(select_sql, &client).await;
            assert_eq!(rows.len(), 1, "{reset}");
            assert_eq!(rows[0]["i"]["c"], "encrypted_text", "{reset}");

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

            // With mapping enabled, an encrypted write after a rolled-back reset
            // still uses the default keyset
            client.simple_query("BEGIN").await.unwrap();
            client.simple_query(reset).await.unwrap();
            client.simple_query("ROLLBACK").await.unwrap();
            client.execute("BEGIN", &[]).await.unwrap();
            client.execute(reset, &[]).await.unwrap();
            client.execute("ROLLBACK", &[]).await.unwrap();

            let id = random_id();
            client.execute(sql, &[&id, &encrypted_text]).await.unwrap();
            assert_eq!(
                stored_keyset_id(id).await,
                Some(default_keyset_id()),
                "{reset}"
            );
            client
                .execute("DELETE FROM encrypted WHERE id = $1", &[&id])
                .await
                .unwrap();
        }
    }

    /// The keyset Proxy is configured to encrypt with when none is selected
    fn default_keyset_id() -> Uuid {
        std::env::var("CS_DEFAULT_KEYSET_ID")
            .or_else(|_| std::env::var("CS_ENCRYPT__DEFAULT_KEYSET_ID"))
            .unwrap()
            .parse()
            .unwrap()
    }

    /// The keyset of the `encrypted_text` ciphertext stored for `id`, read from PostgreSQL
    /// directly
    async fn stored_keyset_id(id: i64) -> Option<Uuid> {
        let rows = query_direct_by::<Value>(
            "SELECT encrypted_text::jsonb FROM encrypted WHERE id = $1",
            &id,
        )
        .await;
        let [row] = rows.as_slice() else {
            panic!("expected one row for {id}, got {rows:?}");
        };
        match serde_json::from_value(row.clone()).unwrap() {
            EqlCiphertextV3::Encrypted(payload) => payload.ciphertext.keyset_id,
            EqlCiphertextV3::SteVec(_) => panic!("expected a scalar ciphertext, got {row}"),
        }
    }
}
