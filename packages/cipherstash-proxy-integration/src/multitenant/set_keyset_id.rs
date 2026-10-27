/// IMPORTANT
/// IMPORTANT
///
/// These test assumes that `CS_DEFAULT_KEYSET_ID` has not been set
///
/// The mise integration task splits the `multitenant` tests so that the config can be changed
///
#[cfg(test)]
mod tests {
    use crate::common::{
        assert_db_error, clear, connect_with_tls, random_id, rows_to_vec, simple_query_with_client,
        trace, PROXY,
    };
    use uuid::Uuid;

    /// Pipelined SET LOCAL must be applied before the dependent Bind is encrypted,
    /// so ciphertext and search terms use the transaction's local keyset.
    #[tokio::test]
    async fn pipelined_set_local_encrypts_bound_parameters_with_the_local_keyset() {
        trace();
        clear().await;

        let local_keyset = std::env::var("CS_TENANT_KEYSET_ID_2")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();
        // This session value is deliberately unavailable. If Bind used it instead of the
        // SET LOCAL sent before it, encryption would fail.
        let session_keyset = Uuid::new_v4();
        assert_ne!(session_keyset, local_keyset);
        let client = connect_with_tls(PROXY).await;
        client
            .batch_execute(&format!("SET CIPHERSTASH.KEYSET_ID = '{session_keyset}'"))
            .await
            .unwrap();
        let begin = client.prepare("BEGIN").await.unwrap();
        let local = client
            .prepare(&format!(
                "SET LOCAL CIPHERSTASH.KEYSET_ID = '{local_keyset}'"
            ))
            .await
            .unwrap();
        let insert = client
            .prepare("INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)")
            .await
            .unwrap();
        let id = random_id();
        let value = 4922_i32;
        let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[&id, &value];

        // Each future is polled in wire order before any response is awaited.
        // Prepared inputs ensure the dependent request already contains Bind.
        let (began, changed, inserted) = tokio::join!(
            biased;
            client.execute(&begin, &[]),
            client.execute(&local, &[]),
            client.execute(&insert, params),
        );
        began.unwrap();
        changed.unwrap();
        assert_eq!(inserted.unwrap(), 1);
        client.batch_execute("COMMIT").await.unwrap();

        // A fresh connection must match the local tenant's encrypted search
        // terms and decrypt the value, independently of the writer's state.
        let reader = connect_with_tls(PROXY).await;
        reader
            .batch_execute(&format!("SET CIPHERSTASH.KEYSET_ID = '{local_keyset}'"))
            .await
            .unwrap();
        let row = reader
            .query_one(
                "SELECT encrypted_int4 FROM encrypted WHERE id = $1 AND encrypted_int4 = $2",
                &[&id, &value],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, i32>(0), value);
    }

    /// A keyset selected earlier in a simple query encrypts the literals after it, and the
    /// SET still reaches PostgreSQL when Proxy rewrites the query.
    #[tokio::test]
    async fn set_keyset_id_and_insert_in_one_simple_query() {
        trace();
        clear().await;

        let keyset = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();
        let id = random_id();
        let value = 5073_i32;

        let client = connect_with_tls(PROXY).await;
        client
            .simple_query(&format!(
                "SET CIPHERSTASH.KEYSET_ID = '{keyset}'; \
                 INSERT INTO encrypted (id, encrypted_int4) VALUES ({id}, {value})"
            ))
            .await
            .unwrap();

        let shown = simple_query_with_client::<String>("SHOW cipherstash.keyset_id", &client).await;
        assert_eq!(shown, vec![keyset.to_string()]);

        let row = client
            .query_one(
                "SELECT encrypted_int4 FROM encrypted WHERE id = $1 AND encrypted_int4 = $2",
                &[&id, &value],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, i32>(0), value);
    }

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

        clear().await;

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
    /// Tests error handling of unknown keyset id
    ///
    #[tokio::test]
    async fn set_keyset_id_unknown() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();
        // Can set unknown name
        let sql = "SET CIPHERSTASH.KEYSET_ID = '2cace9db-3a2a-4b46-a184-ba412b3e0730'";
        let result = client.query(sql, &[]).await;
        assert!(result.is_ok());

        // Error on encrypt
        let id = random_id();
        let text = "TEST UNKNOWN";

        let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        let result = client.query(insert_sql, &[&id, &text]).await;

        assert_db_error(
            result,
            "FATAL",
            "Unknown keyset name or id '2cace9db-3a2a-4b46-a184-ba412b3e0730'. Check the configured credentials. For help visit https://github.com/cipherstash/proxy/blob/main/docs/errors.md#encrypt-unknown-keyset",
        );

        //  --------
        // Switch back to TENANT_1
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");
        let result = client.query(&sql, &[]).await;
        assert!(result.is_ok());

        let insert_sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        let result = client.query(insert_sql, &[&id, &text]).await;
        assert!(result.is_ok());
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
            "SET CIPHERSTASH.KEYSET_ID = NULL".to_string(),
            "SET CIPHERSTASH.KEYSET_ID = 123".to_string(),
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

    ///
    /// Tests that a connection pool's reset clears the keyset, so that one client's keyset is
    /// not carried over to the next client of a pooled connection.
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn pool_reset_clears_keyset_id() {
        trace();
        clear().await;

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let insert_sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        let set_sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");

        let client = connect_with_tls(PROXY).await;

        for reset in [
            "DISCARD ALL",
            "RESET ALL",
            "RESET CIPHERSTASH.KEYSET_ID",
            "SET CIPHERSTASH.KEYSET_ID TO DEFAULT",
        ] {
            // Simple query
            client.simple_query(&set_sql).await.unwrap();
            client.query(insert_sql, &[&random_id(), &1]).await.unwrap();
            client.simple_query(reset).await.unwrap();

            // CS_DEFAULT_KEYSET_ID is not set for these tests, so a keyset is required again
            let result = client.query(insert_sql, &[&random_id(), &1]).await;
            assert!(result.is_err(), "{reset}");

            // Extended query
            client.execute(&set_sql, &[]).await.unwrap();
            client.query(insert_sql, &[&random_id(), &1]).await.unwrap();
            client.execute(reset, &[]).await.unwrap();

            let result = client.query(insert_sql, &[&random_id(), &1]).await;
            assert!(result.is_err(), "{reset}");
        }
    }

    ///
    /// Tests that the keyset follows PostgreSQL's transactions: a reset in a rolled-back
    /// transaction keeps the keyset, and a rolled-back `SET` is undone.
    /// Test both extended and simple query protocols
    ///
    #[tokio::test]
    async fn rolled_back_keyset_changes_are_undone() {
        trace();
        clear().await;

        let tenant_keyset_id_1 = std::env::var("CS_TENANT_KEYSET_ID_1")
            .map(|s| Uuid::parse_str(&s).unwrap())
            .unwrap();

        let insert_sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        let set_sql = format!("SET CIPHERSTASH.KEYSET_ID = '{tenant_keyset_id_1}'");

        let client = connect_with_tls(PROXY).await;
        client.simple_query(&set_sql).await.unwrap();

        for reset in [
            "RESET ALL",
            "RESET CIPHERSTASH.KEYSET_ID",
            "SET CIPHERSTASH.KEYSET_ID TO DEFAULT",
        ] {
            // Simple query
            client.simple_query("BEGIN").await.unwrap();
            client.simple_query(reset).await.unwrap();
            client.simple_query("ROLLBACK").await.unwrap();

            // The keyset is still selected
            let result = client.query(insert_sql, &[&random_id(), &1]).await;
            assert!(result.is_ok(), "{reset}");

            // Extended query
            client.execute("BEGIN", &[]).await.unwrap();
            client.execute(reset, &[]).await.unwrap();
            client.execute("ROLLBACK", &[]).await.unwrap();

            let result = client.query(insert_sql, &[&random_id(), &1]).await;
            assert!(result.is_ok(), "{reset}");
        }

        client.simple_query("RESET ALL").await.unwrap();

        // DEFAULT_KEYSET_ID IS DISABLED FOR THIS TEST, SO A ROLLED-BACK SET LEAVES NO KEYSET
        client.simple_query("BEGIN").await.unwrap();
        client.simple_query(&set_sql).await.unwrap();
        client.simple_query("ROLLBACK").await.unwrap();

        let result = client.query(insert_sql, &[&random_id(), &1]).await;
        assert!(result.is_err());

        client.execute("BEGIN", &[]).await.unwrap();
        client.execute(&set_sql, &[]).await.unwrap();
        client.execute("ROLLBACK", &[]).await.unwrap();

        let result = client.query(insert_sql, &[&random_id(), &1]).await;
        assert!(result.is_err());
    }
}
