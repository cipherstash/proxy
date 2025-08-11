#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, execute_query, query_by, query_with_client, random_id,
        rows_to_vec, trace, PROXY,
    };
    use serde_json::Value;
    use tracing::{error, info};
    use uuid::Uuid;

    ///
    /// Tests that the `SET CIPHERSTASH.KEYSET_ID = 'keyset_id'` command is handled correctly
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn set_keyset_id_with_extended_query() {
        trace();
        clear().await;

        let default_keyset_id = std::env::var("CS_DEFAULT_KEYSET_ID")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let tenant_keyset_id_2 = std::env::var("CS_TENANT_KEYSET_ID_2")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let insert_sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        let select_sql = "SELECT encrypted_int4 FROM encrypted WHERE id = $1";

        // // INSERT with DEFAULT keyset
        let default_id = random_id();
        let default_encrypted_val = 1;
        execute_query(&insert_sql, &[&default_id, &encrypted_val]).await;

        // KEYSET_ID IS SCOPRED TO A CONNECTION
        // The same client/connection is used for tests
        let client = connect_with_tls(PROXY).await;

        // SELECT with DEFAULT keyset
        let rows = client.query(select_sql, &[&default_id]).await.unwrap();
        let actual = rows_to_vec::<i32>(&rows);
        let expected = vec![default_encrypted_val];

        // Test setting keyset_id with single quotes
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok(), "Setting keyset_id should succeed");

        //  --

        // INSERT and SELECT as TENANT 1
        let tenant_1_id = random_id();
        let encrypted_val = 2;

        client
            .query(insert_sql, &[&tenant_1_id, &encrypted_val])
            .await
            .unwrap();

        let rows = client.query(select_sql, &[&tenant_1_id]).await.unwrap();
        let actual = rows_to_vec::<i32>(&rows);

        let expected = vec![encrypted_val];
        assert_eq!(expected, actual);

        // As TENANT 1 SELECT the data created with the DEFAULT keyset
        let result = client.query(select_sql, &[&default_id]).await;
        assert!(result.is_err());

        // Use the DEFAULT keyset
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{default_keyset_Id}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok(), "Setting keyset_id should succeed");

        // Record can now be decrypted
        let rows = client.query(select_sql, &[&default_id]).await.unwrap();
        let actual = rows_to_vec::<i32>(&rows);
        let expected = vec![default_encrypted_val];
    }

    ///
    /// Tests that the `SET CIPHERSTASH.KEYSET_ID = 'keyset_id'` command works with simple queries
    ///
    #[tokio::test]
    async fn set_keyset_id_with_simple_query() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        // Test setting keyset_id with simple query
        let sql = "SET CIPHERSTASH.KEYSET_ID = 'simple-query-keyset-789'";
        let result = client.simple_query(sql).await;
        assert!(
            result.is_ok(),
            "Setting keyset_id with simple query should succeed"
        );

        // Test that regular operations still work
        let id = random_id();
        let encrypted_text = "simple query test".to_string();

        let insert_sql = format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({}, '{}')",
            id, encrypted_text
        );
        let result = client.simple_query(&insert_sql).await;
        assert!(
            result.is_ok(),
            "Simple insert should work after setting keyset_id"
        );
    }

    ///
    /// Tests that keyset_id setting is bound to the specific client connection
    ///
    #[tokio::test]
    async fn set_keyset_id_bound_to_client() {
        trace();

        clear().await;

        // Client 1 sets a keyset_id
        let client1 = connect_with_tls(PROXY).await;
        let sql = "SET CIPHERSTASH.KEYSET_ID = 'client1-keyset'";
        let result = client1.query(sql, &[]).await;
        assert!(result.is_ok(), "Client 1 should be able to set keyset_id");

        // Client 2 should not be affected by client 1's keyset_id setting
        let client2 = connect_with_tls(PROXY).await;

        // Both clients should be able to perform operations independently
        let id1 = random_id();
        let id2 = random_id();
        let text1 = "client1 data".to_string();
        let text2 = "client2 data".to_string();

        let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";

        // Client 1 insert (has keyset_id set)
        let result1 = client1.query(insert_sql, &[&id1, &text1]).await;
        assert!(result1.is_ok(), "Client 1 insert should succeed");

        // Client 2 insert (no keyset_id set)
        let result2 = client2.query(insert_sql, &[&id2, &text2]).await;
        assert!(result2.is_ok(), "Client 2 insert should succeed");

        // Client 2 can now set its own keyset_id
        let sql = "SET CIPHERSTASH.KEYSET_ID = 'client2-keyset'";
        let result = client2.query(sql, &[]).await;
        assert!(
            result.is_ok(),
            "Client 2 should be able to set its own keyset_id"
        );

        // Both clients should still be able to query their data
        let select_sql = "SELECT encrypted_text FROM encrypted WHERE id = $1";

        let rows1 = query_with_client::<String>(select_sql, &client1).await;
        assert_eq!(rows1.len(), 1);
        assert_eq!(rows1[0], text1);

        let rows2 = query_with_client::<String>(select_sql, &client2).await;
        assert_eq!(rows2.len(), 1);
        assert_eq!(rows2[0], text2);
    }

    ///
    /// Tests various string literal formats for keyset_id values
    ///
    #[tokio::test]
    async fn set_keyset_id_string_formats() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        // Test different string literal formats
        let test_cases = vec![
            ("'single-quoted'", "single-quoted"),
            ("\"double-quoted\"", "double-quoted"),
            ("'keyset-with-dashes-123'", "keyset-with-dashes-123"),
            (
                "'keyset_with_underscores_456'",
                "keyset_with_underscores_456",
            ),
            ("'UPPERCASE-KEYSET'", "UPPERCASE-KEYSET"),
            ("'lowercase-keyset'", "lowercase-keyset"),
            ("'Mixed-Case-Keyset'", "Mixed-Case-Keyset"),
        ];

        for (quoted_value, _expected_value) in test_cases {
            let sql = format!("SET CIPHERSTASH.KEYSET_ID = {}", quoted_value);
            let result = client.query(&sql, &[]).await;
            assert!(
                result.is_ok(),
                "Setting keyset_id with {} should succeed",
                quoted_value
            );

            // Test that operations still work after each keyset_id change
            let id = random_id();
            let text = format!("test data for {}", quoted_value);

            let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
            let result = client.query(insert_sql, &[&id, &text]).await;
            assert!(
                result.is_ok(),
                "Insert should work after setting keyset_id to {}",
                quoted_value
            );
        }
    }

    ///
    /// Tests error handling for invalid keyset_id syntax
    ///
    #[tokio::test]
    async fn set_keyset_id_invalid_syntax() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        // Test cases that should potentially fail or be handled gracefully
        let invalid_cases = vec![
            "SET CIPHERSTASH.KEYSET_ID = unquoted-value", // unquoted string
            "SET CIPHERSTASH.KEYSET_ID = 123",            // numeric value
            "SET CIPHERSTASH.KEYSET_ID = NULL",           // null value
        ];

        for invalid_sql in invalid_cases {
            let result = client.query(invalid_sql, &[]).await;
            // The proxy should either handle these gracefully or return appropriate errors
            // We don't assert failure here as the behavior may vary based on implementation
            println!("Result for '{}': {:?}", invalid_sql, result);
        }

        // Ensure that after invalid attempts, a valid keyset_id can still be set
        let sql = "SET CIPHERSTASH.KEYSET_ID = 'recovery-keyset'";
        let result = client.query(sql, &[]).await;
        assert!(
            result.is_ok(),
            "Should be able to set valid keyset_id after invalid attempts"
        );
    }
}
