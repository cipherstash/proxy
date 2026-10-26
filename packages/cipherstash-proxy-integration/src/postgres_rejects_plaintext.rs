#[cfg(test)]
/// PostgreSQL's EQL domain checks as the last guard against plaintext.
///
/// Proxy routes a prepared statement once, at Parse, and a transaction type
/// checks against the schema pinned when it started. DDL committed by another
/// connection in between can make a column encrypted after Proxy has decided
/// to forward a statement that writes to it, and Proxy then forwards the value
/// as written. PostgreSQL is then what stops the plaintext: an EQL v3 column
/// is a domain over `jsonb` whose CHECK only accepts an EQL payload. The check
/// runs only when a value is stored, so these tests cover writes, not reads.
///
/// Each test also writes a JSON-shaped plaintext, which parses as `jsonb`, so
/// that the domain CHECK, not `jsonb` parsing, is shown to reject it.
mod tests {
    use crate::common::{connect_for_test, get_database_port, random_id, PROXY};
    use serde_json::Value;
    use tokio_postgres::error::SqlState;
    use tokio_postgres::types::Type;
    use tokio_postgres::Client;

    /// A plaintext that is valid `jsonb`, so only the domain CHECK can reject it.
    const JSON_PLAINTEXT: &str = r#"{"name": "alice"}"#;

    /// Plaintexts written as SQL literals, each with the rejection
    /// PostgreSQL gives it.
    const PLAINTEXTS: [(&str, Rejection); 2] = [
        (JSON_PLAINTEXT, Rejection::DomainCheck),
        ("plaintext", Rejection::InvalidJson),
    ];

    /// Why PostgreSQL rejected a plaintext written to an EQL column.
    #[derive(Clone, Copy, Debug)]
    enum Rejection {
        /// The value is `jsonb`, but not an EQL payload.
        DomainCheck,
        /// The value is not `jsonb` at all.
        InvalidJson,
    }

    impl Rejection {
        fn code(self) -> SqlState {
            match self {
                Rejection::DomainCheck => SqlState::CHECK_VIOLATION,
                Rejection::InvalidJson => SqlState::INVALID_TEXT_REPRESENTATION,
            }
        }

        fn message_prefix(self) -> &'static str {
            match self {
                Rejection::DomainCheck => {
                    "value for domain eql_v3_text_search violates check constraint"
                }
                Rejection::InvalidJson => "invalid input syntax for type json",
            }
        }
    }

    fn table_name(prefix: &str) -> String {
        format!("{prefix}_{}", random_id())
    }

    async fn create_native_table(client: &Client, table: &str) {
        client
            .batch_execute(&format!(
                "CREATE TABLE {table} (id bigint PRIMARY KEY, secret jsonb)"
            ))
            .await
            .unwrap();
    }

    async fn create_encrypted_table(client: &Client, table: &str) {
        client
            .batch_execute(&format!(
                "CREATE TABLE {table} (id bigint PRIMARY KEY, secret eql_v3_text_search)"
            ))
            .await
            .unwrap();
    }

    /// Makes the native `secret` column encrypted, as a migration on another
    /// connection would.
    async fn encrypt_column(client: &Client, table: &str) {
        client
            .batch_execute(&format!(
                "ALTER TABLE {table} ALTER COLUMN secret TYPE eql_v3_text_search"
            ))
            .await
            .unwrap();
    }

    /// Asserts that PostgreSQL itself rejected the write, for this reason.
    fn assert_rejected<T: std::fmt::Debug>(
        result: Result<T, tokio_postgres::Error>,
        rejection: Rejection,
    ) {
        let err = result.unwrap_err();
        let db_error = err
            .as_db_error()
            .unwrap_or_else(|| panic!("expected a database error, got: {err:?}"));

        assert_eq!(db_error.code(), &rejection.code(), "{db_error:?}");
        assert!(
            db_error.message().starts_with(rejection.message_prefix()),
            "{rejection:?}: {db_error:?}"
        );
    }

    /// Asserts, directly against PostgreSQL, that no value reached the table.
    async fn assert_nothing_stored(table: &str) {
        let postgres = connect_for_test(get_database_port()).await;
        let sql = format!("SELECT count(*) FROM {table}");
        let count: i64 = postgres.query_one(&sql, &[]).await.unwrap().get(0);

        assert_eq!(count, 0);
    }

    /// Connection A prepares an INSERT while `secret` is native, so Proxy routes
    /// it to be forwarded. Connection B then makes `secret` encrypted and
    /// commits. A's Bind and Execute still use the route chosen at Parse.
    #[tokio::test]
    async fn stale_prepared_statement_is_rejected_by_postgres() {
        let a = connect_for_test(*PROXY).await;
        let b = connect_for_test(*PROXY).await;
        let table = table_name("stale_prepared");
        create_native_table(&a, &table).await;

        let insert = a
            .prepare(&format!("INSERT INTO {table} (id, secret) VALUES ($1, $2)"))
            .await
            .unwrap();
        assert_eq!(insert.params(), &[Type::INT8, Type::JSONB]);

        encrypt_column(&b, &table).await;

        let plaintext: Value = serde_json::from_str(JSON_PLAINTEXT).unwrap();
        let result = a.execute(&insert, &[&1_i64, &plaintext]).await;
        assert_rejected(result, Rejection::DomainCheck);

        assert_nothing_stored(&table).await;
    }

    /// Connection A starts a transaction and touches an unrelated table, which
    /// pins its schema while `secret` is native. Connection B then makes
    /// `secret` encrypted and commits. A's INSERT is checked against the
    /// pinned schema, so Proxy forwards its literal as written.
    #[tokio::test]
    async fn insert_in_pinned_transaction_is_rejected_by_postgres() {
        for (plaintext, rejection) in PLAINTEXTS {
            let a = connect_for_test(*PROXY).await;
            let b = connect_for_test(*PROXY).await;
            let table = table_name("pinned_snapshot");
            let unrelated = table_name("pinned_snapshot_unrelated");
            create_native_table(&a, &table).await;
            create_native_table(&a, &unrelated).await;

            a.batch_execute("BEGIN").await.unwrap();
            a.simple_query(&format!("SELECT count(*) FROM {unrelated}"))
                .await
                .unwrap();

            encrypt_column(&b, &table).await;

            let result = a
                .simple_query(&format!(
                    "INSERT INTO {table} (id, secret) VALUES (1, '{plaintext}')"
                ))
                .await;
            assert_rejected(result, rejection);
            a.batch_execute("ROLLBACK").await.unwrap();

            assert_nothing_stored(&table).await;
        }
    }

    /// A statement prepared while `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` is on
    /// is forwarded, and Proxy keeps no analysis of it. Unlike the cases above,
    /// PostgreSQL is not the guard here: Proxy refuses the Bind once mapping is
    /// turned back on, because the setting changed since the Parse.
    #[tokio::test]
    async fn statement_prepared_with_mapping_disabled_is_refused_after_mapping_is_enabled() {
        let a = connect_for_test(*PROXY).await;
        let table = table_name("disabled_mapping_prepared");
        create_encrypted_table(&a, &table).await;

        a.batch_execute("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
            .await
            .unwrap();
        // Typed as `jsonb`, the domain's base type, so the client can send a
        // value; PostgreSQL would otherwise describe the parameter as the domain.
        let insert = a
            .prepare_typed(
                &format!("INSERT INTO {table} (id, secret) VALUES ($1, $2)"),
                &[Type::INT8, Type::JSONB],
            )
            .await
            .unwrap();
        a.batch_execute("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false")
            .await
            .unwrap();

        let plaintext: Value = serde_json::from_str(JSON_PLAINTEXT).unwrap();
        let err = a.execute(&insert, &[&1_i64, &plaintext]).await.unwrap_err();
        let db_error = err
            .as_db_error()
            .unwrap_or_else(|| panic!("expected a database error, got: {err:?}"));
        assert_eq!(
            db_error.code(),
            &SqlState::INVALID_SQL_STATEMENT_NAME,
            "{db_error:?}"
        );
        assert!(
            db_error
                .message()
                .starts_with("The prepared statement or bound values no longer match"),
            "{db_error:?}"
        );

        assert_nothing_stored(&table).await;
    }
}
