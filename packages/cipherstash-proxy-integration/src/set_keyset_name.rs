#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, execute_query, query_with_client, random_id, rows_to_vec, trace,
        PROXY,
    };

    ///
    /// Tests that the `SET CIPHERSTASH.KEYSET_NAME = 'keyset_name'` command is handled correctly
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn set_keyset_name_with_extended_query() {
        trace();
        clear().await;

        let default_keyset_name = "default";
        let tenant_keyset_name_1 = "tenant-1";
        let _tenant_keyset_name_2 = "tenant-2";

        let insert_sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        let select_sql = "SELECT encrypted_int4 FROM encrypted WHERE id = $1";

        //  --------
        // INSERT with DEFAULT keyset name
        let default_id = random_id();
        let default_encrypted_val = 1;
        execute_query(insert_sql, &[&default_id, &default_encrypted_val]).await;

        // KEYSET_NAME IS SCOPED TO A CONNECTION
        // The same client/connection is used for tests
        let client = connect_with_tls(PROXY).await;

        // SELECT with DEFAULT keyset name
        let rows = client.query(select_sql, &[&default_id]).await.unwrap();
        let actual = rows_to_vec::<i32>(&rows);
        let expected = vec![default_encrypted_val];
        assert_eq!(expected, actual);

        //  --------
        // INSERT and SELECT as TENANT 1

        // SET tenant 1
        let sql = format!("SET CIPHERSTASH.KEYSET_NAME = '{tenant_keyset_name_1}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok(), "Setting keyset_name should succeed");

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

        //  --------
        // SELECT data created with DEFAULT as TENANT 1
        let result = client.query(select_sql, &[&default_id]).await;
        assert!(result.is_err());

        //  --------
        // SELECT as DEFAULT

        let sql = format!("SET CIPHERSTASH.KEYSET_NAME = '{default_keyset_name}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok(), "Setting keyset_name should succeed");

        let rows = client.query(select_sql, &[&default_id]).await.unwrap();
        let actual = rows_to_vec::<i32>(&rows);
        let expected = vec![default_encrypted_val];
        assert_eq!(expected, actual);
    }

    ///
    /// Tests that the `SET CIPHERSTASH.KEYSET_NAME = 'keyset_name'` command works with simple queries
    ///
    #[tokio::test]
    async fn set_keyset_name_with_simple_query() {
        trace();

        clear().await;

        let tenant_keyset_name_1 = "tenant-1";

        let client = connect_with_tls(PROXY).await;

        // Test setting keyset_name with simple query
        let sql = format!("SET CIPHERSTASH.KEYSET_NAME = '{tenant_keyset_name_1}'");
        let result = client.simple_query(&sql).await;
        assert!(
            result.is_ok(),
            "Setting keyset_name with simple query should succeed"
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
            "Simple insert should work after setting keyset_name"
        );
    }

    ///
    /// Tests that keyset_name setting is bound to the specific client connection
    ///
    #[tokio::test]
    async fn set_keyset_name_bound_to_client() {
        trace();

        clear().await;

        let default_keyset_name = "default";
        let tenant_keyset_name_1 = "tenant-1";
        let tenant_keyset_name_2 = "tenant-2";

        // Client 1 sets a keyset_name
        let client1 = connect_with_tls(PROXY).await;
        let sql = format!("SET CIPHERSTASH.KEYSET_NAME = '{tenant_keyset_name_1}'");
        let result = client1.query(&sql, &[]).await;
        assert!(result.is_ok(), "Client 1 should be able to set keyset_name");

        // Client 2 should not be affected by client 1's keyset_name setting
        let client2 = connect_with_tls(PROXY).await;

        // Both clients should be able to perform operations independently
        let id1 = random_id();
        let id2 = random_id();
        let text1 = "client1 data".to_string();
        let text2 = "client2 data".to_string();

        let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";

        // Client 1 insert (has keyset_name set)
        let result1 = client1.query(insert_sql, &[&id1, &text1]).await;
        assert!(result1.is_ok(), "Client 1 insert should succeed");

        // Client 2 insert (no keyset_name set)
        let result2 = client2.query(insert_sql, &[&id2, &text2]).await;
        assert!(result2.is_ok(), "Client 2 insert should succeed");

        //  --------
        // Client 2 can now set its own keyset_name
        let sql = format!("SET CIPHERSTASH.KEYSET_NAME = '{tenant_keyset_name_2}'");
        let result = client2.query(&sql, &[]).await;
        assert!(result.is_ok());

        //  --------
        // SELECT with Client 1
        let sql = format!("SELECT encrypted_text FROM encrypted WHERE id = {id1}");
        let rows = query_with_client::<String>(&sql, &client1).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], text1);

        // SELECT with Client 2 (which is now TENANT 2) is an error
        let sql = format!("SELECT encrypted_text FROM encrypted WHERE id = {id2}");
        let result = client2.query(&sql, &[]).await;
        assert!(result.is_err());

        //  --------
        // Client 2 sets DEFAULT
        let sql = format!("SET CIPHERSTASH.KEYSET_NAME = '{default_keyset_name}'");
        let result = client2.query(&sql, &[]).await;
        assert!(result.is_ok());

        // SELECT with Client 2 (which is now DEFAULT) should succeed
        let sql = format!("SELECT encrypted_text FROM encrypted WHERE id = {id2}");
        let rows = query_with_client::<String>(&sql, &client2).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], text2);
    }

    ///
    /// Tests various string literal formats for keyset_name values
    ///
    #[tokio::test]
    async fn set_keyset_name_string_formats() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let tenant_keyset_name_1 = "tenant-1";

        // Test different string literal formats
        let test_cases = vec![
            format!("'{tenant_keyset_name_1}'"), // single quote
            format!("'{}'", tenant_keyset_name_1.to_lowercase()),
            format!("'{}'", tenant_keyset_name_1.to_uppercase()),
            format!("'{}'", tenant_keyset_name_1.replace("-", "")),
        ];

        for value in test_cases {
            let sql = format!("SET CIPHERSTASH.KEYSET_NAME = {}", value);
            let result = client.query(&sql, &[]).await;
            assert!(result.is_ok());

            // Test that operations still work after each keyset_name change
            let id = random_id();
            let text = format!("test data for {}", value);

            let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
            let result = client.query(insert_sql, &[&id, &text]).await;
            assert!(result.is_ok());
        }
    }

    ///
    /// Tests error handling for invalid keyset_name syntax
    ///
    #[tokio::test]
    async fn set_keyset_name_invalid_syntax() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let tenant_keyset_name_1 = "tenant-1";

        // Test cases that should potentially fail or be handled gracefully
        let invalid_cases = vec![
            format!("SET CIPHERSTASH.KEYSET_NAME = {tenant_keyset_name_1}"), // unquoted string
            format!("SET CIPHERSTASH.KEYSET_NAME = \"{tenant_keyset_name_1}\""), // double quoted string
            format!("SET CIPHERSTASH.KEYSET_NAME = 123"),                        // numeric value
            format!("SET CIPHERSTASH.KEYSET_NAME = NULL"),                       // null value
        ];

        for invalid_sql in invalid_cases {
            let result = client.query(&invalid_sql, &[]).await;
            assert!(result.is_err());
        }

        // Ensure that after invalid attempts, a valid keyset_name can still be set
        let sql = format!("SET CIPHERSTASH.KEYSET_NAME = '{tenant_keyset_name_1}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok());
    }
}
