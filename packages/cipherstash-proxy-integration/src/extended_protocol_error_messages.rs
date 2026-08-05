#[cfg(test)]
mod tests {
    use tracing::{debug, info};

    use crate::common::{clear, connect_with_tls, random_id, reset_schema, trace, PROXY};

    /// A statement that always fails inside the proxy, at Parse, in every
    /// configuration: the proxy's SQL parser rejects it before it reaches the
    /// server (same shape as [`invalid_sql_statement`]).
    ///
    /// A transformation failure (e.g. equality on the storage-only
    /// `eql_v3_boolean`) cannot be used here: type-check errors only surface
    /// when `CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS` is on, and the CI proxy
    /// (like production) runs with it off, silently passing such statements
    /// through. A parse error takes the same failure path in the proxy
    /// (`handle_statement_error`) regardless of that flag.
    const FAILS_IN_PROXY: &str = "INSERT INTO encrypted id, encrypted_text VALUES ($1, $2)";

    struct Reset;

    impl Drop for Reset {
        fn drop(&mut self) {
            debug!("Reset schema");
            tokio::spawn(async {
                reset_schema().await;
                debug!("Reset schema complete");
            });
        }
    }

    #[tokio::test]
    async fn encrypted_column_not_defined_in_schema() {
        trace();

        clear().await;

        let _reset = Reset;

        let id = random_id();

        let client = connect_with_tls(*PROXY).await;

        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_unconfigured) VALUES ($1, $2)";
        let result = client.query(sql, &[&id, &encrypted_text]).await;

        assert!(result.is_err());

        if let Err(err) = result {
            let msg = err.to_string();
            assert_eq!(msg, "db error: ERROR: column \"encrypted_unconfigured\" of relation \"encrypted\" does not exist");
        } else {
            unreachable!();
        }
    }

    /// A storage-only encrypted column round-trips.
    ///
    /// Under EQL v2 this asserted the opposite: `unconfigured.encrypted_unconfigured`
    /// was an `eql_v2_encrypted` column with no matching row in
    /// `eql_v2_configuration`, so it was encrypted-but-unconfigured and the proxy
    /// rejected writes with `EncryptUnknownColumn`.
    ///
    /// v3 makes that state unreachable. A column is encrypted precisely because
    /// it has an EQL domain type, and every recognised domain yields a
    /// `ColumnConfig` — `eql_v3_text` simply yields one with no search indexes.
    /// "Encrypted but unconfigured" no longer exists as a condition, so the
    /// column is now a working storage-only column: encrypted on write, decrypted
    /// on read, with no searchable terms.
    #[tokio::test]
    async fn storage_only_encrypted_column_round_trips() {
        trace();

        reset_schema().await;

        let client = connect_with_tls(*PROXY).await;

        let _reset = Reset;

        let id = random_id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO unconfigured (id, encrypted_unconfigured) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "SELECT encrypted_unconfigured FROM unconfigured WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        let actual: String = rows[0].get("encrypted_unconfigured");
        assert_eq!(encrypted_text, actual);
    }

    /// The error here is in the Tokio/Postgres layer
    /// The statement is valid and parses correctly, and the encrypted_date columns is Described as a date
    /// An i32 cannot be converted to a date and tokio_postgres returns an error
    /// See python tests for example with no client type checking
    #[tokio::test]
    async fn mapper_unsupported_parameter_type_with_date() {
        trace();

        let client = connect_with_tls(*PROXY).await;

        let id = random_id();
        // let encrypted_date = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();
        let encrypted_date: i32 = 2025;

        let sql = "INSERT INTO encrypted (id, encrypted_date) VALUES ($1, $2)";
        let result = client.query(sql, &[&id, &encrypted_date]).await;

        assert!(result.is_err());

        if let Err(err) = result {
            let msg = err.to_string();
            assert_eq!(msg, "error serializing parameter 1: cannot convert between the Rust type `i32` and the Postgres type `date`");
        } else {
            unreachable!();
        }
    }

    /// CIP-3678 regression: a statement that fails inside the proxy on a
    /// connection that has already run a MAPPED (encrypted) statement must
    /// surface the proxy's own error as a clean `db error` — not desync the
    /// extended-protocol stream into a client-side protocol error
    /// (`unexpected message from server`) — and the connection must remain
    /// usable afterwards.
    #[tokio::test]
    async fn proxy_error_after_mapped_statement() {
        trace();

        let client = connect_with_tls(*PROXY).await;

        // Mapped warm-up: an encrypted statement that parses, binds and
        // executes successfully.
        client
            .query(
                "SELECT id FROM encrypted WHERE encrypted_text = $1",
                &[&"cip-3678"],
            )
            .await
            .unwrap();

        // A statement that fails inside the proxy must return the proxy's
        // error, delivered as a database error.
        let err = client
            .query(FAILS_IN_PROXY, &[&random_id(), &"cip-3678"])
            .await
            .unwrap_err();
        let db_err = err.as_db_error().unwrap_or_else(|| {
            panic!("expected a db error carrying the proxy's message, got: {err:?}")
        });
        assert!(
            db_err.message().contains("sql parser error"),
            "expected the proxy's parse error, got: {db_err:?}"
        );

        // The connection must remain usable.
        let rows = client.query("SELECT 1::int4", &[]).await.unwrap();
        let one: i32 = rows[0].get(0);
        assert_eq!(one, 1);
    }

    /// Companion to [`proxy_error_after_mapped_statement`]: the same failing
    /// statement on a connection that has only run passthrough statements.
    /// This path already worked; keep it covered.
    #[tokio::test]
    async fn proxy_error_after_passthrough_statement() {
        trace();

        let client = connect_with_tls(*PROXY).await;

        // Passthrough warm-up.
        client.query("SELECT 1::int4", &[]).await.unwrap();

        let err = client
            .query(FAILS_IN_PROXY, &[&random_id(), &"cip-3678"])
            .await
            .unwrap_err();
        let db_err = err.as_db_error().unwrap_or_else(|| {
            panic!("expected a db error carrying the proxy's message, got: {err:?}")
        });
        assert!(
            db_err.message().contains("sql parser error"),
            "expected the proxy's parse error, got: {db_err:?}"
        );

        // The connection must remain usable.
        let rows = client.query("SELECT 1::int4", &[]).await.unwrap();
        let one: i32 = rows[0].get(0);
        assert_eq!(one, 1);
    }

    #[tokio::test]
    async fn invalid_sql_statement() {
        trace();

        reset_schema().await;

        let client = connect_with_tls(*PROXY).await;

        let _reset = Reset;

        // Create a record
        // If select returns no results, no configuration is required
        let id = random_id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted id, encrypted_text VALUES ($1, $2)";
        let result = client.query(sql, &[&id, &encrypted_text]).await;

        assert!(result.is_err());

        if let Err(err) = result {
            let msg = err.to_string();
            info!("{}", msg);
            assert_eq!(msg, "db error: ERROR: sql parser error: Expected: SELECT, VALUES, or a subquery in the query body, found: id at Line: 1, Column: 23. For help visit https://github.com/cipherstash/proxy/blob/main/docs/errors.md#mapping-invalid-sql-statement");
        } else {
            unreachable!();
        }
    }
}
