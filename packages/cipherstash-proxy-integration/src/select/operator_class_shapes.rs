//! Shapes that group, sort or deduplicate an encrypted column through
//! PostgreSQL's **operator class** rather than through an operator.
//!
//! EQL v3 overloads `=`, `<`, `<=`, `>`, `>=` for its domains, which is why
//! `=`, `IN`, `BETWEEN` and `IS DISTINCT FROM` are correct with no rewriting at
//! all (see [`super::operator_backed_predicates`]).
//!
//! Sorting, grouping and deduplication do not go through those operators. They
//! use the type's default btree/hash operator class, which for a jsonb-backed
//! domain is jsonb's — and jsonb compares whole payloads, starting at `c`, the
//! ciphertext, which is randomised per encryption. Every row therefore looks
//! distinct and every sort order is arbitrary.
//!
//! That is exactly why `ORDER BY col`, `GROUP BY col` and `SELECT DISTINCT col`
//! are rewritten to their term functions. The shapes below reach the same
//! operator class by a route no rule covers yet, so they are still wrong — and
//! wrong *silently*, with no error to say the clause was not applied.
//!
//! # These tests are ignored, not deleted
//!
//! Each asserts the behaviour the shape must have. They fail today; un-ignoring
//! one is the acceptance test for its fix. Rejecting the shape loudly at
//! type-check time is an equally acceptable outcome — in which case the test
//! should be rewritten to assert the error, not deleted.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, execute_query, random_id, trace, PROXY};

    /// Five rows over three distinct plaintexts, so a failure to deduplicate or
    /// group shows up as a count.
    async fn insert_fixture() {
        for (text, int4) in [
            ("apple", 1),
            ("banana", 2),
            ("cherry", 3),
            ("apple", 4),
            ("banana", 5),
        ] {
            let id = random_id();
            execute_query(
                "INSERT INTO encrypted (id, encrypted_text, encrypted_int4) VALUES ($1, $2, $3)",
                &[&id, &text.to_string(), &int4],
            )
            .await;
        }
    }

    fn sorted(mut values: Vec<String>) -> Vec<String> {
        values.sort();
        values
    }

    /// `DISTINCT ON (enc)` keeps one row per distinct value of `enc`.
    ///
    /// `SELECT DISTINCT enc` is rewritten to key on the equality term, but
    /// `DISTINCT ON (enc)` written by the client is passed through, so it
    /// deduplicates on the raw payload and keeps every row.
    #[tokio::test]
    #[ignore = "DISTINCT ON (<encrypted column>) is not rewritten: it deduplicates on the raw \
                jsonb payload, whose ciphertext is randomised, so no rows collapse. Needs the \
                same eq_term keying SELECT DISTINCT already gets, or a loud rejection."]
    async fn distinct_on_encrypted_column_deduplicates() {
        trace();
        clear().await;
        insert_fixture().await;

        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT DISTINCT ON (encrypted_text) encrypted_text FROM encrypted";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();

        assert_eq!(vec!["apple", "banana", "cherry"], sorted(actual));
    }

    /// `ORDER BY 1` sorts by the first projected column, encrypted or not.
    ///
    /// The ordinal is left untouched, so PostgreSQL sorts the payload by jsonb
    /// rules rather than by the column's ordering term.
    #[tokio::test]
    #[ignore = "ORDER BY <ordinal> referring to an encrypted column is not rewritten: the ordinal \
                is left as-is, so the sort falls back to jsonb ordering over the randomised \
                ciphertext and the order is arbitrary. RewriteEqlOrderBy only handles named \
                columns."]
    async fn order_by_ordinal_sorts_by_the_encrypted_column() {
        trace();
        clear().await;
        insert_fixture().await;

        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT encrypted_text FROM encrypted ORDER BY 1";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();

        assert_eq!(vec!["apple", "apple", "banana", "banana", "cherry"], actual);
    }

    /// `GROUP BY 1` groups by the first projected column, encrypted or not.
    #[tokio::test]
    #[ignore = "GROUP BY <ordinal> referring to an encrypted column is not rewritten: the ordinal \
                is left as-is, so grouping happens on the raw payload and every row becomes its \
                own group. RewriteEqlGroupBy only handles named columns."]
    async fn group_by_ordinal_groups_by_the_encrypted_column() {
        trace();
        clear().await;
        insert_fixture().await;

        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT encrypted_text FROM encrypted GROUP BY 1";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();

        assert_eq!(vec!["apple", "banana", "cherry"], sorted(actual));
    }

    /// A window partitioned by an encrypted column groups equal plaintexts.
    #[tokio::test]
    #[ignore = "PARTITION BY <encrypted column> is not rewritten — no rule covers a window \
                specification — so partitioning happens on the raw payload and every row lands \
                in its own partition, making every rank 1."]
    async fn window_partition_by_encrypted_column_groups_equal_plaintexts() {
        trace();
        clear().await;
        insert_fixture().await;

        let client = connect_with_tls(PROXY).await;

        // Two 'apple' rows and two 'banana' rows, so each of those partitions
        // must produce a rank 2. Every rank being 1 means no partitioning.
        let sql = "SELECT rank() OVER (PARTITION BY encrypted_text ORDER BY encrypted_int4) AS r \
                   FROM encrypted";
        let rows = client.query(sql, &[]).await.unwrap();
        let mut ranks: Vec<i64> = rows.iter().map(|r| r.get("r")).collect();
        ranks.sort();

        assert_eq!(vec![1, 1, 1, 2, 2], ranks);
    }

    /// A deduplicating set operation on an encrypted column is refused.
    ///
    /// Deduplication compares whole payloads, so `UNION` would keep every
    /// duplicate. It cannot be keyed on the equality term in place — the
    /// comparison spans both branches' whole projections — so it is rejected
    /// rather than silently wrong. `UNION ALL` deduplicates nothing and works.
    #[tokio::test]
    #[ignore = "The rejection is a type-check error, and with CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS \
                unset — the default, and what the proxy container runs with, since \
                tests/docker-compose.yml does not pass it through — a type error falls back to \
                passthrough. The statement then reaches PostgreSQL unrewritten and returns the \
                un-deduplicated rows this test exists to prevent. The mapper-level test \
                (deduplicating_set_operations_on_encrypted_columns_are_rejected) covers the \
                rejection itself; this becomes runnable once mapping errors are always on \
                (CIP-3680)."]
    async fn deduplicating_set_operations_are_rejected() {
        trace();
        clear().await;
        insert_fixture().await;

        let client = connect_with_tls(PROXY).await;

        for sql in [
            "SELECT encrypted_text FROM encrypted UNION SELECT encrypted_text FROM encrypted",
            "SELECT encrypted_text FROM encrypted INTERSECT SELECT encrypted_text FROM encrypted",
            "SELECT encrypted_text FROM encrypted EXCEPT SELECT encrypted_text FROM encrypted",
        ] {
            let err = client
                .query(sql, &[])
                .await
                .expect_err(&format!("`{sql}` should be refused"));

            assert!(
                err.to_string()
                    .contains("deduplication would compare ciphertexts"),
                "unexpected error for `{sql}`: {err}"
            );
        }

        // UNION ALL keeps duplicates by definition, so it is unaffected.
        let sql =
            "SELECT encrypted_text FROM encrypted UNION ALL SELECT encrypted_text FROM encrypted";
        let rows = client.query(sql, &[]).await.unwrap();
        assert_eq!(
            10,
            rows.len(),
            "UNION ALL should return both branches in full"
        );
    }
}
