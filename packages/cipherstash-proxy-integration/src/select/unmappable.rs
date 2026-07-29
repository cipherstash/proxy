#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, get_database_port, random_id, PROXY};
    use std::error::Error;

    ///
    /// Tests that a statement Proxy cannot map is refused.
    ///
    /// There is no configuration that turns this off. A statement that fails to type check and
    /// touches an encrypted column is always an error, because forwarding it produces a wrong
    /// answer rather than a degraded one.
    ///
    #[tokio::test]
    async fn unmappable_table_not_found() {
        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT blah FROM vtha";
        let result = client.query(sql, &[]).await;

        assert!(
            result.is_err(),
            "Expected unmappble SQL statement to return an error",
        );
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
    /// `eql_v3_boolean` is storage-only: it carries no equality term, so `DISTINCT` cannot be keyed
    /// on it and the statement fails to type check.
    ///
    /// Proxy used to swallow that failure and forward the statement, which returned the raw EQL
    /// payloads — `{"c": "mBbK<n$E_kDWiD#g9BY2...", "i": {...}, "v": 3}` — straight to the client
    /// with no error at all. It must be refused instead.
    ///
    #[tokio::test]
    async fn select_distinct_on_a_storage_only_column_is_refused() {
        clear().await;

        let client = connect_with_tls(*PROXY).await;

        let id = random_id();
        let sql = "INSERT INTO encrypted (id, encrypted_bool) VALUES ($1, $2)";
        client.query(sql, &[&id, &true]).await.unwrap();

        // A fresh connection, so the assertion is about the refusal itself and not about how the
        // driver frames an error arriving on a connection it has already used.
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
    /// `requires_type_check` is purely syntactic, so every `SELECT` is type checked whether or not
    /// encryption is involved, and the mapper's SQL coverage is narrower than PostgreSQL's. Driver
    /// introspection of `pg_catalog` fails to type check (`Table not found: pg_catalog.pg_type`)
    /// and essentially every PostgreSQL driver issues it. Rejecting it would break working
    /// applications for no security benefit — there is no encrypted data in the statement.
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

    ///
    /// The same rule applied to an ordinary query over a table with no encrypted columns:
    /// `ARRAY_AGG`/`CARDINALITY` cannot be typed by the mapper, but the statement is harmless.
    ///
    #[tokio::test]
    async fn unmappable_native_only_statement_is_forwarded() {
        clear().await;

        let client = connect_with_tls(*PROXY).await;

        let id = random_id();
        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &"hello@cipherstash.com"])
            .await
            .unwrap();

        let sql = "SELECT ARRAY_REMOVE(ARRAY_AGG(id), NULL), plaintext
                     FROM plaintext
                    WHERE CARDINALITY(ARRAY[1,2]) <> 0
                    GROUP BY plaintext";
        let rows = client.query(sql, &[]).await.unwrap();

        assert_eq!(rows.len(), 1);
    }

    ///
    /// A statement over a table the schema has never heard of has no encrypted columns to expose,
    /// so it is forwarded and PostgreSQL rejects it with its own error rather than Proxy inventing
    /// one. Clients depend on seeing the real database error.
    ///
    #[tokio::test]
    async fn unknown_table_is_reported_by_postgres_not_proxy() {
        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT * FROM blahvtha";
        let result = client.query(sql, &[]).await;

        match result {
            Ok(_) => panic!("Expected an error for an unknown table"),
            Err(error) => {
                let db_error = error.source().unwrap().to_string();
                assert_eq!(db_error, "ERROR: relation \"blahvtha\" does not exist");
            }
        }
    }

    ///
    /// A read that Proxy cannot map must not fall back to handing the client raw EQL payloads.
    ///
    #[tokio::test]
    async fn unmappable_read_does_not_leak_ciphertext_to_the_client() {
        clear().await;

        let client = connect_with_tls(*PROXY).await;

        let id = random_id();
        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &"hello@cipherstash.com"])
            .await
            .unwrap();

        // Native and encrypted cannot be unified, so this cannot be mapped.
        let sql = "SELECT encrypted_text FROM encrypted WHERE plaintext = encrypted_text";
        let result = client.query(sql, &[]).await;

        assert!(
            result.is_err(),
            "Expected an unmappable read of an encrypted column to be refused rather than \
             returning raw EQL payloads",
        );
    }
}
