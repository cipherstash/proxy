#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, random_id, rows_to_vec, simple_query_with_client, trace, PROXY,
    };
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
        let result = client.simple_query(sql).await;
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
        let expected = vec![encrypted_text];

        let sql = format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({}, '{}')",
            tenant_1_id, encrypted_text
        );

        let result = client.simple_query(&sql).await;
        assert!(result.is_ok());

        // SELECT
        let sql = format!("SELECT encrypted_text FROM encrypted WHERE id = {tenant_1_id}");
        let actual = simple_query_with_client::<String>(&sql, &client).await;

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

        // Both clients should be able to perform operations independently
        let tenant_1_id = random_id();
        let tenant_2_id = random_id();
        let tenant_1_text = "TENANT_1".to_string();
        let tenant_2_text = "TENANT_2".to_string();

        let tenant_1_client = connect_with_tls(PROXY).await;
        let tenant_2_client = connect_with_tls(PROXY).await;

        // DEFAULT_KEYSET_ID SHOULD BE DISABLED FOR THIS TEST
        // SET KEYSET IS REQUIRED
        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES (1, 'hello')";
        let result = tenant_1_client.simple_query(sql).await;
        assert!(result.is_err());

        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = tenant_1_client.query(&sql, &[]).await;
        assert!(result.is_ok());

        // TENANT_2 has no keyset, INSERT should fail
        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES (1, 'hello')";
        let result = tenant_2_client.simple_query(sql).await;
        assert!(result.is_err());

        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_2}'");
        let result = tenant_2_client.query(&sql, &[]).await;
        assert!(result.is_ok());

        // ------------------

        let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";

        // TENANT_1 INSERT
        let result = tenant_1_client
            .query(insert_sql, &[&tenant_1_id, &tenant_1_text])
            .await;
        assert!(result.is_ok());

        // TENANT_2 INSERT
        let result = tenant_2_client
            .query(insert_sql, &[&tenant_2_id, &tenant_2_text])
            .await;
        assert!(result.is_ok());

        // ------------------

        // SELECT TENANT_1 record with TENANT_2 should fail
        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {tenant_1_id}");
        let result = tenant_1_client.simple_query(&sql).await;
        assert!(result.is_ok());

        // SELECT TENANT_2 record with TENANT_1 should fail
        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {tenant_2_id}");
        let result = tenant_1_client.simple_query(&sql).await;
        assert!(result.is_err());

        // ------------------

        // SELECT TENANT_2 record with TENANT_1 should fail
        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {tenant_2_id}");
        let result = tenant_2_client.simple_query(&sql).await;
        assert!(result.is_ok());

        // SELECT TENANT_1 record with TENANT_2 should fail
        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {tenant_1_id}");
        let result = tenant_2_client.simple_query(&sql).await;
        assert!(result.is_err());
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
            let text = format!("TEST: {}", value);

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
        let invalid = vec![
            format!("SET CIPHERSTASH.KEYSET_ID = {tenant_keyset_id_1}"), // unquoted string
            format!("SET CIPHERSTASH.KEYSET_ID = NULL"),
            format!("SET CIPHERSTASH.KEYSET_ID = 123"),
        ];

        for sql in invalid {
            let result = client.query(&sql, &[]).await;
            assert!(result.is_err());
        }

        // Ensure that after invalid attempts, a valid keyset_id can still be set
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok());
    }
}
