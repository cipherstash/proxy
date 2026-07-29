#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, get_database_port, random_id, PROXY};
    use std::error::Error;

    ///
    /// A statement Proxy cannot map is refused when it touches an encrypted column. There is no
    /// configuration that turns this off: forwarding such a statement does not degrade the answer,
    /// it makes it wrong — an unmapped read returns raw ciphertext, an unmapped predicate compares
    /// a plaintext literal against a jsonb payload, and an unmapped write stores plaintext.
    ///
    /// `vtha` is not in the schema, so it has no encrypted columns to expose and the statement is
    /// forwarded. The error the client sees is PostgreSQL's own, not one Proxy invented, and that
    /// matters: clients rely on `relation "..." does not exist` to distinguish a missing table from
    /// a proxy fault.
    ///
    #[tokio::test]
    async fn unmappable_table_not_found() {
        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT blah FROM vtha";
        let result = client.query(sql, &[]).await;

        match result {
            Ok(_) => panic!("Expected an error for an unknown table"),
            Err(error) => {
                let db_error = error.source().unwrap().to_string();
                assert_eq!(db_error, "ERROR: relation \"vtha\" does not exist");
            }
        }
    }

    #[tokio::test]
    async fn unmappable_column_not_found() {
        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT blah FROM encrypted";
        let result = client.query(sql, &[]).await;

        assert!(
            result.is_err(),
            "Expected unmappble SQL statement to return an error",
        );
    }

    #[tokio::test]
    async fn unmappable_native_cannot_be_unified_with_encrypted() {
        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT * FROM encrypted WHERE plaintext = encrypted_text";
        let result = client.query(sql, &[]).await;

        assert!(
            result.is_err(),
            "Expected unmappble SQL statement to return an error",
        );
    }

    #[tokio::test]
    async fn unmappable_syntax_error() {
        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT *, FROM encrypted";
        let result = client.query(sql, &[]).await;

        assert!(
            result.is_err(),
            "Expected unmappble SQL statement to return an error",
        );
    }

    ///
    /// The reproducer for CIP-3680.
    ///
    /// `eql_v3_boolean` is storage-only: it carries no equality term, so `DISTINCT` cannot be keyed
    /// on it and the statement fails to type check.
    ///
    /// Proxy used to swallow that failure and forward the statement, which returned the raw EQL
    /// payloads — `{"c": "mBbK<n$E_kDWiD#g9BY2...", "i": {...}, "v": 3}` — straight to the client
    /// with no error at all. It must be refused instead.
    ///
    /// The connection is fresh, which is load bearing — see
    /// `select_distinct_on_a_storage_only_column_is_refused_on_a_reused_connection`.
    ///
    #[tokio::test]
    async fn select_distinct_on_a_storage_only_column_is_refused() {
        clear().await;

        let setup = connect_with_tls(*PROXY).await;
        let id = random_id();
        let sql = "INSERT INTO encrypted (id, encrypted_bool) VALUES ($1, $2)";
        setup.query(sql, &[&id, &true]).await.unwrap();

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT DISTINCT encrypted_bool FROM encrypted";
        let result = client.query(sql, &[]).await;

        let err = match result {
            Ok(rows) => panic!(
                "Expected DISTINCT on a storage-only encrypted column to be refused, \
                 but Proxy returned {} row(s) of raw ciphertext",
                rows.len()
            ),
            Err(err) => err,
        };

        let message = match err.source() {
            Some(db_error) => db_error.to_string(),
            None => err.to_string(),
        };

        assert!(
            message.contains("could not be type checked"),
            "Expected a mapping error, got: {message}"
        );
    }

    ///
    /// The same refusal on a connection that has already run a statement — which is every
    /// connection in a pool, and so the case that actually matters in production.
    ///
    /// The statement is still refused, and that is the security property this file exists to pin.
    /// But the client cannot read *why*: it gets tokio-postgres' `unexpected message from server`
    /// instead of the mapping error, and the connection is broken rather than reusable. The
    /// assertion is deliberately weak here so the test documents the defect instead of asserting
    /// it away.
    ///
    /// The cause is an ordering bug in Proxy, not in this test, and it is older than the removal
    /// of `enable_mapping_errors` — that flag simply kept the path from being reached by default.
    /// Frontend and backend are separate tasks writing to one unbounded channel, so a synthetic
    /// ErrorResponse raised on the frontend is queued immediately and overtakes responses the
    /// server still owes for earlier messages. Observed byte order for
    /// `Close(s0) Sync | Parse(s1) Describe(s1) Sync`:
    ///
    /// ```text
    /// sent:     ErrorResponse, ReadyForQuery, CloseComplete, ReadyForQuery
    /// expected: CloseComplete, ReadyForQuery, ErrorResponse, ReadyForQuery
    /// ```
    ///
    /// The client is waiting for `CloseComplete` and gets `ErrorResponse`, so it gives up. Fixing
    /// it needs the frontend to hold a synthetic error until the backend has drained what the
    /// server owes, which is a change to the proxy's concurrency model and not in scope here.
    ///
    #[tokio::test]
    async fn select_distinct_on_a_storage_only_column_is_refused_on_a_reused_connection() {
        clear().await;

        let client = connect_with_tls(*PROXY).await;

        let id = random_id();
        let sql = "INSERT INTO encrypted (id, encrypted_bool) VALUES ($1, $2)";
        client.query(sql, &[&id, &true]).await.unwrap();

        let sql = "SELECT DISTINCT encrypted_bool FROM encrypted";
        let result = client.query(sql, &[]).await;

        if let Ok(rows) = result {
            panic!(
                "Expected DISTINCT on a storage-only encrypted column to be refused, \
                 but Proxy returned {} row(s) of raw ciphertext",
                rows.len()
            );
        }
    }

    ///
    /// The strongest form of the assertion: a statement Proxy refuses must never reach PostgreSQL.
    ///
    /// This `INSERT` selects a native `text` column into an encrypted column, which cannot be
    /// unified. Proxy used to forward it unmapped, sending the value to the database without
    /// encrypting it. The check is made against PostgreSQL *directly*, bypassing Proxy, so nothing
    /// in the read path can disguise a write that landed.
    ///
    #[tokio::test]
    async fn unmappable_write_to_an_encrypted_column_never_reaches_postgres() {
        clear().await;

        let client = connect_with_tls(*PROXY).await;

        let id = random_id();
        let plaintext = "hello@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id, &plaintext]).await.unwrap();

        // Native `plaintext.plaintext` cannot be unified with encrypted `encrypted.encrypted_text`.
        let sql = "INSERT INTO encrypted (id, encrypted_text) SELECT id, plaintext FROM plaintext WHERE id = $1";
        let result = client.query(sql, &[&id]).await;

        assert!(
            result.is_err(),
            "Expected an unmappable write to an encrypted column to be refused",
        );

        // Ask the database itself, not Proxy.
        let db = connect_with_tls(get_database_port()).await;
        let rows = db
            .query("SELECT encrypted_text FROM encrypted WHERE id = $1", &[&id])
            .await
            .unwrap();

        assert!(
            rows.is_empty(),
            "Refused statement still wrote {} row(s) to the database",
            rows.len()
        );
    }

    ///
    /// The counterpart, and the reason the unmappable check is narrowed to statements touching an
    /// encrypted column rather than made fatal outright.
    ///
    /// `requires_type_check` is purely syntactic, so every query is type checked whether or not
    /// encryption is involved, and the mapper's SQL coverage is narrower than PostgreSQL's.
    /// Introspection of `pg_catalog` fails to type check (`Table not found: pg_catalog.pg_type`),
    /// and it is not an exotic thing to issue: `psql`'s `\d`, `\dt` and `\l` are all this shape,
    /// and tokio-postgres itself issues one to resolve the OID of an EQL v3 domain. Rejecting them
    /// would break working applications for no security benefit — there is no encrypted data in
    /// the statement to get wrong.
    ///
    /// `passthrough::tests::passthrough_select_with_cardinality` covers the same rule for an
    /// ordinary query over a table with no encrypted columns.
    ///
    #[tokio::test]
    async fn unmappable_statement_with_no_encrypted_columns_is_forwarded() {
        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT attname, atttypid FROM pg_catalog.pg_attribute WHERE attnum > 0 LIMIT 5";
        let rows = client.query(sql, &[]).await.unwrap();

        assert!(
            !rows.is_empty(),
            "Expected pg_catalog introspection to still be forwarded to the database",
        );
    }
}
