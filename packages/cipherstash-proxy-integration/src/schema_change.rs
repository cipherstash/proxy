#[cfg(test)]
/// End-to-end schema-change tests through Proxy and directly against PostgreSQL.
mod tests {
    use crate::common::{
        configure_test_client, connect, connect_with_tls, connection_config, get_database_port,
        random_id, PROXY,
    };
    use std::sync::Arc;
    use tokio::sync::Barrier;
    use tokio::time::{timeout, Duration, Instant};
    use tokio_postgres::Client;

    async fn connect_for_test(port: u16) -> Client {
        if std::env::var("CS_TEST_USE_TLS").as_deref() == Ok("false") {
            connect(port).await
        } else {
            connect_with_tls(port).await
        }
    }

    async fn connect_for_disconnect_test(port: u16) -> (Client, tokio::task::JoinHandle<()>) {
        let config = connection_config(port);
        if std::env::var("CS_TEST_USE_TLS").as_deref() == Ok("false") {
            let (client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
            let task = tokio::spawn(async move {
                let _ = connection.await;
            });
            (client, task)
        } else {
            let tls = tokio_postgres_rustls::MakeRustlsConnect::new(configure_test_client());
            let (client, connection) = config.connect(tls).await.unwrap();
            let task = tokio::spawn(async move {
                let _ = connection.await;
            });
            (client, task)
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

    async fn wait_until_alter_is_blocked(postgres: &Client, table: &str) {
        wait_for_alter_lock_state(postgres, table, true).await;
    }

    async fn wait_for_alter_lock_state(postgres: &Client, table: &str, expected: bool) {
        let sql = "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE relation = to_regclass($1) AND NOT granted)";
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let blocked: bool = postgres.query_one(sql, &[&table]).await.unwrap().get(0);
            if blocked == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "ALTER TABLE lock state did not become {expected}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    #[tokio::test]
    async fn concurrent_commits_are_both_visible_after_readiness() {
        let observer = connect_for_test(*PROXY).await;
        let first = connect_for_test(*PROXY).await;
        let second = connect_for_test(*PROXY).await;
        let first_table = table("bug_308_concurrent_first");
        let second_table = table("bug_308_concurrent_second");
        first.batch_execute("BEGIN").await.unwrap();
        second.batch_execute("BEGIN").await.unwrap();
        first
            .execute(&create_encrypted_table(&first_table), &[])
            .await
            .unwrap();
        second
            .execute(&create_encrypted_table(&second_table), &[])
            .await
            .unwrap();

        let barrier = Arc::new(Barrier::new(3));
        let first_commit = tokio::spawn({
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                first.batch_execute("COMMIT").await.unwrap()
            }
        });
        let second_commit = tokio::spawn({
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                second.batch_execute("COMMIT").await.unwrap()
            }
        });
        barrier.wait().await;
        first_commit.await.unwrap();
        second_commit.await.unwrap();

        insert_secret(&observer, &first_table, 1, "first concurrent secret").await;
        insert_secret(&observer, &second_table, 2, "second concurrent secret").await;
        assert_ciphertext_at_rest(&first_table, 1, "first concurrent secret").await;
        assert_ciphertext_at_rest(&second_table, 2, "second concurrent secret").await;
    }

    #[tokio::test]
    async fn cancelling_blocked_ddl_does_not_activate_a_schema_change() {
        timeout(Duration::from_secs(20), async {
            let postgres = connect_for_test(get_database_port()).await;
            let proxy = connect_for_test(*PROXY).await;
            let table = table("bug_308_cancel");
            proxy
                .batch_execute(&format!("CREATE TABLE {table} (id bigint)"))
                .await
                .unwrap();
            postgres
                .batch_execute(&format!(
                    "BEGIN; LOCK TABLE {table} IN ACCESS SHARE MODE"
                ))
                .await
                .unwrap();

            let cancel = proxy.cancel_token();
            let alter = format!("ALTER TABLE {table} ADD COLUMN secret eql_v3_text_search");
            let task = tokio::spawn(async move { proxy.batch_execute(&alter).await });
            wait_until_alter_is_blocked(&postgres, &table).await;
            if std::env::var("CS_TEST_USE_TLS").as_deref() == Ok("false") {
                cancel.cancel_query(tokio_postgres::NoTls).await.unwrap();
            } else {
                let tls = tokio_postgres_rustls::MakeRustlsConnect::new(configure_test_client());
                cancel.cancel_query(tls).await.unwrap();
            }
            assert!(task.await.unwrap().is_err());
            postgres.batch_execute("ROLLBACK").await.unwrap();

            let fresh = connect_for_test(*PROXY).await;
            assert_eq!(fresh.execute(&format!("INSERT INTO {table} (id) VALUES (1)"), &[]).await.unwrap(), 1);
            let exists: bool = postgres.query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = $1 AND column_name = 'secret')",
                &[&table],
            ).await.unwrap().get(0);
            assert!(!exists);
        }).await.unwrap();
    }

    #[tokio::test]
    async fn disconnecting_during_blocked_ddl_does_not_cancel_the_upstream_schema_change() {
        timeout(Duration::from_secs(20), async {
            let postgres = connect_for_test(get_database_port()).await;
            let (proxy, connection_task) = connect_for_disconnect_test(*PROXY).await;
            let table = table("bug_308_disconnect");
            proxy
                .batch_execute(&format!("CREATE TABLE {table} (id bigint)"))
                .await
                .unwrap();
            postgres
                .batch_execute(&format!(
                    "BEGIN; LOCK TABLE {table} IN ACCESS SHARE MODE"
                ))
                .await
                .unwrap();

            let alter = format!("ALTER TABLE {table} ADD COLUMN secret eql_v3_text_search");
            let task = tokio::spawn(async move { proxy.batch_execute(&alter).await });
            wait_until_alter_is_blocked(&postgres, &table).await;
            connection_task.abort();
            assert!(connection_task.await.unwrap_err().is_cancelled());
            assert!(task.await.unwrap().is_err());

            // Proxy has already forwarded the statement to PostgreSQL. Losing
            // the client connection does not cancel that upstream execution.
            postgres.batch_execute("ROLLBACK").await.unwrap();

            let fresh = connect_for_test(*PROXY).await;
            assert_eq!(fresh.execute(&format!("INSERT INTO {table} (id) VALUES (1)"), &[]).await.unwrap(), 1);
            let exists: bool = postgres.query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = $1 AND column_name = 'secret')",
                &[&table],
            ).await.unwrap().get(0);
            assert!(exists);
        }).await.unwrap();
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
    async fn encryption_neutral_alter_table_keeps_transaction_mappable() {
        let client = connect_for_test(*PROXY).await;
        let table = table("bug_308_safe_alter");

        client
            .execute(&create_encrypted_table(&table), &[])
            .await
            .unwrap();
        client.batch_execute("BEGIN").await.unwrap();
        client
            .batch_execute(&format!(
                "ALTER TABLE {table} ALTER COLUMN secret SET NOT NULL"
            ))
            .await
            .unwrap();
        insert_secret(&client, &table, 1, "after safe alter").await;
        client.batch_execute("COMMIT").await.unwrap();

        assert_ciphertext_at_rest(&table, 1, "after safe alter").await;
    }

    #[tokio::test]
    async fn native_temporary_table_remains_usable_without_blocking_encrypted_mapping() {
        let client = connect_for_test(*PROXY).await;
        let temporary = table("bug_308_native_temp");
        let encrypted = table("bug_308_after_native_temp");

        client
            .batch_execute(&format!(
                "CREATE TEMPORARY TABLE {temporary} (id bigint PRIMARY KEY, name text); \
                 INSERT INTO {temporary} (id, name) VALUES (1, 'temporary')"
            ))
            .await
            .unwrap();
        let name: String = client
            .query_one(&format!("SELECT name FROM {temporary} WHERE id = 1"), &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(name, "temporary");

        client
            .execute(&create_encrypted_table(&encrypted), &[])
            .await
            .unwrap();
        insert_secret(&client, &encrypted, 1, "after native temporary table").await;
        assert_ciphertext_at_rest(&encrypted, 1, "after native temporary table").await;
    }

    #[tokio::test]
    async fn schema_qualified_ddl_shares_identity_with_bare_name_references() {
        let client = connect_for_test(*PROXY).await;
        let table = table("bug_308_qualified");

        client.batch_execute("BEGIN").await.unwrap();
        client
            .execute(
                &format!(
                    "CREATE TABLE public.{table} \
                     (id bigint PRIMARY KEY, secret eql_v3_text_search NOT NULL)"
                ),
                &[],
            )
            .await
            .unwrap();
        insert_secret(&client, &table, 1, "qualified secret").await;
        client.batch_execute("COMMIT").await.unwrap();

        assert_ciphertext_at_rest(&table, 1, "qualified secret").await;
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
            .batch_execute("SAVEPOINT Before_Reverted")
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

    #[tokio::test]
    async fn compatibility_fallback_tracks_native_ddl_for_a_later_encrypted_alter() {
        let client = connect_for_test(*PROXY).await;
        let table = table("bug_308_fallback");

        client
            .batch_execute(&format!(
                "CREATE TABLE {table} (id bigint PRIMARY KEY); \
                 INSERT INTO {table} (id) VALUES (1)"
            ))
            .await
            .unwrap();
        client
            .execute(
                &format!("ALTER TABLE {table} ADD COLUMN secret eql_v3_text_search"),
                &[],
            )
            .await
            .unwrap();

        insert_secret(&client, &table, 2, "after fallback").await;
        assert_ciphertext_at_rest(&table, 2, "after fallback").await;
    }

    #[tokio::test]
    async fn rewritten_simple_query_preserves_native_ddl_and_its_intent() {
        let client = connect_for_test(*PROXY).await;
        let encrypted = table("bug_308_rewritten");
        let staging = table("bug_308_staging");

        client
            .execute(&create_encrypted_table(&encrypted), &[])
            .await
            .unwrap();
        client
            .batch_execute(&format!(
                "CREATE TABLE {staging} (id bigint PRIMARY KEY); \
                 INSERT INTO {encrypted} (id, secret) VALUES (1, 'preserved batch secret')"
            ))
            .await
            .unwrap();

        let postgres = connect_for_test(get_database_port()).await;
        let exists: bool = postgres
            .query_one("SELECT to_regclass($1) IS NOT NULL", &[&staging])
            .await
            .unwrap()
            .get(0);
        assert!(exists);
        assert_ciphertext_at_rest(&encrypted, 1, "preserved batch secret").await;
    }

    #[tokio::test]
    async fn multi_statement_simple_query_keeps_the_connection_usable() {
        let client = connect_for_test(*PROXY).await;
        let first = table("execution_lifecycle_first");
        let second = table("execution_lifecycle_second");

        client
            .batch_execute(&format!(
                "CREATE TABLE {first} (id bigint); CREATE TABLE {second} (id bigint)"
            ))
            .await
            .unwrap();

        let value: i32 = client.query_one("SELECT 1", &[]).await.unwrap().get(0);
        assert_eq!(value, 1);
    }
}
