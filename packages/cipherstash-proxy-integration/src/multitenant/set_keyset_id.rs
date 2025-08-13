#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, execute_query, query_with_client, random_id, rows_to_vec, trace,
        PROXY,
    };
    use tracing::error;
    use uuid::Uuid;

    ///
    /// Tests that the `SET CIPHERSTASH.KEYSET_ID = 'keyset_id'` command is handled correctly
    ///
    #[tokio::test]
    async fn set_keyset_id_with_extended_query() {
        trace();
        clear().await;

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let tenant_keyset_id_2 = std::env::var("CS_TENANT_KEYSET_ID_2")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let insert_sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        let select_sql = "SELECT encrypted_int4 FROM encrypted WHERE id = $1";

        // KEYSET_ID IS SCOPED TO A CONNECTION
        // The same client/connection is used for tests
        let client = connect_with_tls(PROXY).await;

        // DEFAULT_KEYSET_ID SHOULD BE DISABLED FOR THIS TEST
        // SET KEYSET IS REQUIRED
        let result = client.query(select_sql, &[&42]).await;
        assert!(result.is_err());

        //  --------
        // INSERT and SELECT as TENANT_1
        // SET TENANT_1
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok());

        // INSERT
        let tenant_1_id = random_id();
        let encrypted_val = 2;

        client
            .query(insert_sql, &[&tenant_1_id, &encrypted_val])
            .await
            .unwrap();

        // SELECT
        let rows = client.query(select_sql, &[&tenant_1_id]).await.unwrap();
        let actual = rows_to_vec::<i32>(&rows);

        let expected = vec![encrypted_val];
        assert_eq!(expected, actual);

        //  --------
        // SET TENANT_2
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_2}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok());

        // SELECT data created by TENANT_1 AS TENANT_2
        let result = client.query(select_sql, &[&tenant_1_id]).await;
        assert!(result.is_err());

        //  --------
        // Switch back to TENANT_1
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok());

        // SELECT as TENANT_1
        let result = client.query(select_sql, &[&tenant_1_id]).await;
        assert!(result.is_ok());
    }

    ///
    /// Tests that the `SET CIPHERSTASH.KEYSET_ID = 'keyset_id'` command works with simple queries
    ///
    #[tokio::test]
    async fn set_keyset_id_with_simple_query() {
        trace();

        // clear().await;

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let tenant_keyset_id_2 = std::env::var("CS_TENANT_KEYSET_ID_2")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        // KEYSET_ID IS SCOPED TO A CONNECTION
        // The same client/connection is used for tests
        let client = connect_with_tls(PROXY).await;

        // DEFAULT_KEYSET_ID SHOULD BE DISABLED FOR THIS TEST
        // SET KEYSET IS REQUIRED
        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES (1, 'hello')";
        let result = client.simple_query(&sql).await;
        assert!(result.is_err());

        // // THIS IS COUNTER INTUITIVE
        // // Statement passes through frontend because no mapping is required
        // // Returns without error because there is NO DATA and decrypt is never called
        let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = 42";
        let result = client.simple_query(sql).await;
        assert!(result.is_ok());

        // //  --------
        // INSERT and SELECT as TENANT_1
        // SET TENANT_1
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client.simple_query(&sql).await;
        assert!(result.is_ok());

        // // INSERT
        let tenant_1_id = random_id();
        let encrypted_text = "hello";

        let sql = format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({}, '{}')",
            tenant_1_id, encrypted_text
        );

        let result = client.simple_query(&sql).await;
        assert!(result.is_ok());

        // SELECT
        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {tenant_1_id}");
        let rows = client.simple_query(&sql).await.unwrap();
        let actual = rows_to_vec::<i32>(&rows);

        let expected = vec![encrypted_text];
        assert_eq!(expected, actual);

        //  --------
        // SET TENANT_2
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_2}'");
        let result = client.simple_query(&sql).await;
        assert!(result.is_ok());

        // SELECT data created by TENANT_1 AS TENANT_2
        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {tenant_1_id}");
        let result = client.simple_query(&sql).await;
        assert!(result.is_err());

        //  --------
        // Switch back to TENANT_1
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client.simple_query(&sql).await;
        assert!(result.is_ok());

        // SELECT as TENANT_1
        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {tenant_1_id}");
        let result = client.simple_query(&sql).await;
        assert!(result.is_ok());
    }

    ///
    /// Tests that keyset_id setting is bound to the specific client connection
    ///
    #[tokio::test]
    async fn set_keyset_id_bound_to_client() {
        trace();

        clear().await;

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let tenant_keyset_id_2 = std::env::var("CS_TENANT_KEYSET_ID_2")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        // Client 1 sets a keyset_id
        let client1 = connect_with_tls(PROXY).await;
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client1.query(&sql, &[]).await;
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

        //  --------
        // Client 2 can now set its own keyset_id
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_2}'");
        let result = client2.query(&sql, &[]).await;
        assert!(result.is_ok(),);

        //  --------
        // SELECT with Client 1
        let sql = format!("SELECT encrypted_text FROM encrypted WHERE id = {id1}");
        let rows = query_with_client::<String>(&sql, &client1).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], text1);

        // // SELECT with Client 2 (which is now TENANT 2) is an error
        let sql = format!("SELECT encrypted_text FROM encrypted WHERE id = {id2}");
        let result = client2.query(&sql, &[]).await;
        assert!(result.is_err(),);

        //  --------
        // Client 2 sets DEFAULT
        // let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{default_keyset_id}'");
        // let result = client2.query(&sql, &[]).await;
        // assert!(result.is_ok(),);

        // // SELECT with Client 2 (which is now TENANT 2) is an error
        // let sql = format!("SELECT encrypted_text FROM encrypted WHERE id = {id2}");
        // let rows = query_with_client::<String>(&sql, &client2).await;
        // assert_eq!(rows.len(), 1);
        // assert_eq!(rows[0], text2);
    }

    ///
    /// Tests various string literal formats for keyset_id values
    ///
    #[tokio::test]
    async fn set_keyset_id_string_formats() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        // Test different string literal formats
        let test_cases = vec![
            format!("'{tenant_keyset_id_1}'"), // single quote
            format!("'{}'", tenant_keyset_id_1.to_string().to_lowercase()), // remove dashes from uuid
            format!("'{}'", tenant_keyset_id_1.to_string().to_uppercase()), // remove dashes from uuid
            format!("'{}'", tenant_keyset_id_1.to_string().replace("-", "")), // remove dashes from uuid
        ];

        for value in test_cases {
            let sql = format!("SET CIPHERSTASH.KEYSET_ID = {}", value);
            let result = client.query(&sql, &[]).await;
            assert!(result.is_ok());

            // Test that operations still work after each keyset_id change
            let id = random_id();
            let text = format!("test data for {}", value);

            let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
            let result = client.query(insert_sql, &[&id, &text]).await;
            assert!(result.is_ok(),);
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

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        // Test cases that should potentially fail or be handled gracefully
        let invalid_cases = vec![
            format!("SET CIPHERSTASH.KEYSET_ID = {tenant_keyset_id_1}"), // unquoted string
            format!("SET CIPHERSTASH.KEYSET_ID = \"{tenant_keyset_id_1}\""), // double quoted string
            format!("SET CIPHERSTASH.KEYSET_ID = 123"),                  // numeric value
            format!("SET CIPHERSTASH.KEYSET_ID = NULL"),                 // null value
        ];

        for invalid_sql in invalid_cases {
            let result = client.query(&invalid_sql, &[]).await;
            assert!(result.is_err(),);
        }

        // Ensure that after invalid attempts, a valid keyset_id can still be set
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok(),);
    }
}
