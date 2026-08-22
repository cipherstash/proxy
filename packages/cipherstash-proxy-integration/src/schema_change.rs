#[cfg(test)]
mod tests {
    use crate::common::{connect, connect_with_tls, get_database_port, random_id, PROXY};
    use tokio_postgres::Client;

    async fn connect_for_test(port: u16) -> Client {
        if std::env::var("CS_TEST_USE_TLS").as_deref() == Ok("false") {
            connect(port).await
        } else {
            connect_with_tls(port).await
        }
    }

    fn table(prefix: &str) -> String {
        format!("{prefix}_{}", random_id())
    }

    fn create_encrypted_table(table: &str) -> String {
        format!("CREATE TABLE {table} (id bigint PRIMARY KEY, secret eql_v3_text_search NOT NULL)")
    }

    async fn assert_ciphertext_at_rest(table: &str, id: i64, plaintext: &str) {
        let postgres = connect_for_test(get_database_port()).await;
        let sql = format!("SELECT secret::text FROM {table} WHERE id = $1");
        let stored: String = postgres.query_one(&sql, &[&id]).await.unwrap().get(0);

        assert!(
            !stored.contains(plaintext),
            "plaintext reached PostgreSQL: {stored}"
        );
        let payload: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert!(
            payload.get("c").is_some(),
            "missing record ciphertext: {payload}"
        );
    }

    async fn insert_secret(client: &Client, table: &str, id: i64, plaintext: &str) {
        let sql = format!("INSERT INTO {table} (id, secret) VALUES ($1, $2)");
        assert_eq!(client.execute(&sql, &[&id, &plaintext]).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn later_connection_encrypts_immediately_after_extended_protocol_ddl() {
        let ddl_connection = connect_for_test(*PROXY).await;
        let already_open_connection = connect_for_test(*PROXY).await;
        let table = table("bug_308_extended");

        ddl_connection
            .execute(&create_encrypted_table(&table), &[])
            .await
            .unwrap();

        insert_secret(&already_open_connection, &table, 1, "classified").await;
        assert_ciphertext_at_rest(&table, 1, "classified").await;
    }

    #[tokio::test]
    async fn explicit_transaction_uses_successful_ddl_overlay_before_commit() {
        let client = connect_for_test(*PROXY).await;
        let table = table("bug_308_transaction");

        client.batch_execute("BEGIN").await.unwrap();
        client
            .execute(&create_encrypted_table(&table), &[])
            .await
            .unwrap();
        insert_secret(&client, &table, 1, "inside transaction").await;
        client.batch_execute("COMMIT").await.unwrap();

        assert_ciphertext_at_rest(&table, 1, "inside transaction").await;
    }

    #[tokio::test]
    async fn pipelined_statement_waits_for_extended_ddl_activation() {
        let client = connect_for_test(*PROXY).await;
        let table = table("bug_308_pipeline");
        let create = create_encrypted_table(&table);
        let insert = format!("INSERT INTO {table} (id, secret) VALUES ($1, $2)");
        let create = client.prepare(&create).await.unwrap();

        let (created, inserted) = tokio::join!(
            client.execute(&create, &[]),
            client.execute(&insert, &[&1_i64, &"pipelined"]),
        );
        created.unwrap();
        assert_eq!(inserted.unwrap(), 1);

        assert_ciphertext_at_rest(&table, 1, "pipelined").await;
    }

    #[tokio::test]
    async fn rollback_discards_successful_ddl_overlay() {
        let client = connect_for_test(*PROXY).await;
        let postgres = connect_for_test(get_database_port()).await;
        let table = table("bug_308_rollback");

        client.batch_execute("BEGIN").await.unwrap();
        client
            .execute(&create_encrypted_table(&table), &[])
            .await
            .unwrap();
        client.batch_execute("ROLLBACK").await.unwrap();

        let exists: bool = postgres
            .query_one("SELECT to_regclass($1) IS NOT NULL", &[&table])
            .await
            .unwrap()
            .get(0);
        assert!(!exists);
    }

    #[tokio::test]
    async fn rollback_to_savepoint_restores_schema_and_encryption_overlay() {
        let client = connect_for_test(*PROXY).await;
        let postgres = connect_for_test(get_database_port()).await;
        let retained = table("bug_308_retained");
        let reverted = table("bug_308_reverted");

        client.batch_execute("BEGIN").await.unwrap();
        client
            .execute(&create_encrypted_table(&retained), &[])
            .await
            .unwrap();
        client
            .batch_execute("SAVEPOINT before_reverted")
            .await
            .unwrap();
        client
            .execute(&create_encrypted_table(&reverted), &[])
            .await
            .unwrap();
        client
            .batch_execute("ROLLBACK TO SAVEPOINT before_reverted")
            .await
            .unwrap();
        insert_secret(&client, &retained, 1, "savepoint secret").await;
        client.batch_execute("COMMIT").await.unwrap();

        assert_ciphertext_at_rest(&retained, 1, "savepoint secret").await;
        let exists: bool = postgres
            .query_one("SELECT to_regclass($1) IS NOT NULL", &[&reverted])
            .await
            .unwrap()
            .get(0);
        assert!(!exists);
    }

    #[tokio::test]
    async fn simple_query_batch_with_dependent_post_ddl_statement_fails_closed() {
        let client = connect_for_test(*PROXY).await;
        let postgres = connect_for_test(get_database_port()).await;
        let table = table("bug_308_simple_batch");
        let batch = format!(
            "{}; INSERT INTO {table} (id, secret) VALUES (1, 'plaintext')",
            create_encrypted_table(&table)
        );

        assert!(client.simple_query(&batch).await.is_err());

        let exists: bool = postgres
            .query_one("SELECT to_regclass($1) IS NOT NULL", &[&table])
            .await
            .unwrap()
            .get(0);
        assert!(!exists);
    }
}
