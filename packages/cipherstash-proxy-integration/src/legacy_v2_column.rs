//! A column left behind by a partly-completed EQL v2 -> v3 migration must be
//! refused, not served as plaintext (CIP-3688).
//!
//! The fixture is `encrypted_v2_legacy` in `tests/sql/schema.sql`: a table whose
//! `encrypted_text` column migrated to a v3 domain and whose `encrypted_v2`
//! column is still declared with EQL v2's `eql_v2_encrypted` composite type.
//! That type carries no v3 domain identity, so Proxy can neither encrypt writes
//! to it nor decrypt reads from it.
//!
//! # What these tests actually assert
//!
//! That the client saw an error is the *weak* half of the claim: a statement
//! could be executed and then reported as failed, and the plaintext would still
//! be sitting in PostgreSQL. So every write test here also connects **directly**
//! to PostgreSQL, bypassing Proxy entirely, and asserts the row never landed.
//! That is the assertion the ticket is about.
//!
//! # Why these run with the mapping-error fallback at its default
//!
//! Nothing here sets `CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS`. With that unset —
//! the default, and what the proxy container runs with — a mapper rejection
//! normally falls back to forwarding the original statement to PostgreSQL
//! unchanged, which for this defect would send the plaintext write on its way.
//! This refusal is deliberately exempt from that fallback, and running these
//! tests against a default-configured Proxy is what pins that exemption. If the
//! exemption regressed, these tests would fail even though the mapper still
//! rejected the statement.
#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, random_id, PG_PORT, PROXY};
    use tokio_postgres::Client;

    /// A connection that does not go through Proxy.
    ///
    /// Every claim about what is or is not stored has to be made here. Asking
    /// Proxy what is in the table cannot answer the question, because Proxy
    /// refuses to read the table at all — and even if it did, a decrypting read
    /// would hide the very thing being looked for.
    async fn direct_to_postgres() -> Client {
        connect_with_tls(*PG_PORT).await
    }

    async fn rows_in_fixture(pg: &Client) -> i64 {
        pg.query_one("SELECT count(*) FROM encrypted_v2_legacy", &[])
            .await
            .unwrap()
            .get(0)
    }

    /// Asserts the error is *this* refusal and not some unrelated failure.
    ///
    /// Worth being strict about: "the statement failed" would also be satisfied
    /// by a typo in the test's SQL, which would make the test pass while proving
    /// nothing. The message is also the only thing an operator gets, so its
    /// content — which column, which type, what to do — is part of the contract.
    fn assert_is_the_v2_refusal(err: &tokio_postgres::Error, context: &str) {
        let message = err.to_string();
        let db_message = std::error::Error::source(err)
            .map(|source| source.to_string())
            .unwrap_or(message);

        assert!(
            db_message.contains("encrypted_v2"),
            "{context}: refusal must name the column to migrate, got: {db_message}"
        );
        assert!(
            db_message.contains("eql_v2_encrypted"),
            "{context}: refusal must name the offending type, got: {db_message}"
        );
        assert!(
            db_message.contains("EQL v3"),
            "{context}: refusal must tell the operator to migrate, got: {db_message}"
        );
    }

    /// The write path the ticket is about, naming the legacy column directly.
    #[tokio::test]
    async fn insert_naming_the_legacy_column_is_refused_and_stores_nothing() {
        let client = connect_with_tls(*PROXY).await;
        let pg = direct_to_postgres().await;

        let id = random_id();
        let before = rows_in_fixture(&pg).await;

        let err = client
            .query(
                "INSERT INTO encrypted_v2_legacy (id, encrypted_v2) VALUES ($1, ROW($2)::eql_v2_encrypted)",
                &[&id, &serde_json::json!({"secret": "value"})],
            )
            .await
            .expect_err("INSERT into a legacy EQL v2 column must be refused");

        assert_is_the_v2_refusal(&err, "insert naming the legacy column");

        // The decisive assertion: asked directly, PostgreSQL has no such row.
        assert_eq!(
            rows_in_fixture(&pg).await,
            before,
            "a refused INSERT must not have reached PostgreSQL"
        );
    }

    /// The subtler write: the statement never mentions the legacy column, so a
    /// column-scoped guard would wave it through. It is refused because the
    /// refusal is scoped to the table.
    ///
    /// This is the case that makes the fix fail *closed*. Proving a statement
    /// can never route a value into the legacy column — across `*`, defaults,
    /// triggers, `RETURNING` and rules — is a negative that fails open whenever
    /// it is wrong, and failing open is the bug.
    #[tokio::test]
    async fn insert_avoiding_the_legacy_column_is_still_refused_and_stores_nothing() {
        let client = connect_with_tls(*PROXY).await;
        let pg = direct_to_postgres().await;

        let id = random_id();
        let before = rows_in_fixture(&pg).await;

        let err = client
            .query(
                "INSERT INTO encrypted_v2_legacy (id, plaintext) VALUES ($1, $2)",
                &[&id, &"plaintext that must not be stored"],
            )
            .await
            .expect_err("INSERT into a table with a legacy EQL v2 column must be refused");

        assert_is_the_v2_refusal(&err, "insert avoiding the legacy column");

        assert_eq!(
            rows_in_fixture(&pg).await,
            before,
            "a refused INSERT must not have reached PostgreSQL"
        );
    }

    /// The same write over the simple query protocol, which reaches the type
    /// checker by a different path in the frontend than the extended protocol
    /// above. Both paths have to refuse, and only one of them was exercised by
    /// the test above.
    #[tokio::test]
    async fn simple_protocol_insert_is_refused_and_stores_nothing() {
        let client = connect_with_tls(*PROXY).await;
        let pg = direct_to_postgres().await;

        let id = random_id();
        let before = rows_in_fixture(&pg).await;

        let err = client
            .simple_query(&format!(
                "INSERT INTO encrypted_v2_legacy (id, plaintext) VALUES ({id}, 'leaked')"
            ))
            .await
            .expect_err("simple-protocol INSERT must be refused too");

        assert_is_the_v2_refusal(&err, "simple protocol insert");

        assert_eq!(
            rows_in_fixture(&pg).await,
            before,
            "a refused simple-protocol INSERT must not have reached PostgreSQL"
        );
    }

    /// UPDATE is a write like any other, and the one an operator is most likely
    /// to reach for on a table that already has rows.
    #[tokio::test]
    async fn update_is_refused_and_changes_nothing() {
        let client = connect_with_tls(*PROXY).await;
        let pg = direct_to_postgres().await;

        // Seed a row the only way it can be seeded: behind Proxy's back.
        let id = random_id();
        pg.execute(
            "INSERT INTO encrypted_v2_legacy (id, plaintext) VALUES ($1, $2)",
            &[&id, &"original"],
        )
        .await
        .unwrap();

        let err = client
            .query(
                "UPDATE encrypted_v2_legacy SET plaintext = $1 WHERE id = $2",
                &[&"overwritten", &id],
            )
            .await
            .expect_err("UPDATE on a table with a legacy EQL v2 column must be refused");

        assert_is_the_v2_refusal(&err, "update");

        let still: String = pg
            .query_one(
                "SELECT plaintext FROM encrypted_v2_legacy WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(still, "original", "a refused UPDATE must not have applied");

        pg.execute("DELETE FROM encrypted_v2_legacy WHERE id = $1", &[&id])
            .await
            .unwrap();
    }

    /// The read path.
    ///
    /// Reading back v2 ciphertext is not itself the data-at-rest exposure the
    /// ticket is about — the stored bytes are already encrypted, and returning
    /// them undecrypted leaks nothing. Reads are refused anyway, for two
    /// reasons. First, Proxy cannot decrypt the value, so the alternative is
    /// handing the client ciphertext it will read as data — silently wrong in a
    /// different direction. Second, and decisively, a *predicate* on the column
    /// would send the comparison value to PostgreSQL in the clear, which is a
    /// plaintext exposure on a nominally read-only statement. Refusing the
    /// statement covers both without having to tell them apart.
    #[tokio::test]
    async fn select_is_refused() {
        let client = connect_with_tls(*PROXY).await;

        let err = client
            .query("SELECT id, encrypted_v2 FROM encrypted_v2_legacy", &[])
            .await
            .expect_err("SELECT of a legacy EQL v2 column must be refused");

        assert_is_the_v2_refusal(&err, "select of the legacy column");
    }

    /// A read that never names the legacy column, refused for the same
    /// table-scoped reason as the equivalent INSERT.
    #[tokio::test]
    async fn select_avoiding_the_legacy_column_is_still_refused() {
        let client = connect_with_tls(*PROXY).await;

        let err = client
            .query("SELECT id FROM encrypted_v2_legacy", &[])
            .await
            .expect_err("SELECT on a table with a legacy EQL v2 column must be refused");

        assert_is_the_v2_refusal(&err, "select avoiding the legacy column");
    }

    /// A predicate on the legacy column is the read-path case that is a genuine
    /// exposure: unrefused, the comparison value travels to PostgreSQL in the
    /// clear and appears in its logs and statistics.
    #[tokio::test]
    async fn select_with_a_predicate_on_the_legacy_column_is_refused() {
        let client = connect_with_tls(*PROXY).await;

        let err = client
            .query(
                "SELECT id FROM encrypted_v2_legacy WHERE encrypted_v2 = ROW($1)::eql_v2_encrypted",
                &[&serde_json::json!({"secret": "value"})],
            )
            .await
            .expect_err("a predicate on a legacy EQL v2 column must be refused");

        assert_is_the_v2_refusal(&err, "select with a predicate on the legacy column");
    }

    /// The blast radius stays bounded to the unmigrated table.
    ///
    /// This is the argument for refusing per statement rather than refusing to
    /// start: one column left behind must not take a whole deployment offline,
    /// including for the tables nobody has a problem with.
    #[tokio::test]
    async fn tables_without_a_legacy_column_are_unaffected() {
        let client = connect_with_tls(*PROXY).await;

        let id = random_id();
        client
            .query(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &"still works"],
            )
            .await
            .unwrap();

        let rows = client
            .query("SELECT encrypted_text FROM encrypted WHERE id = $1", &[&id])
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        let value: String = rows[0].get("encrypted_text");
        assert_eq!(value, "still works");

        client
            .query("DELETE FROM encrypted WHERE id = $1", &[&id])
            .await
            .unwrap();
    }
}
