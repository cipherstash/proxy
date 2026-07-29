//! `SELECT DISTINCT … ORDER BY <encrypted column>`.
//!
//! Ordering an encrypted column requires its ordering term, but PostgreSQL
//! requires every `ORDER BY` expression under `DISTINCT` to appear in the select
//! list — and the term does not. The mapper resolves this by pushing the select
//! into a subquery that also projects the term, and ordering the (non-`DISTINCT`)
//! outer query by it.
//!
//! `DISTINCT` itself also has to be rewritten: deduplicating on the raw payload
//! compares randomised ciphertext, so equal plaintexts never collapse. The
//! mapper keys the `DISTINCT` on the column's equality term instead.
//!
//! These tests assert what those rewrites have to get right: equal plaintexts
//! collapse, the rows come back in the correct plaintext order, and the ordering
//! term the subquery projects does not leak into the client's result set.

#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, execute_query, random_id, simple_query, trace, PROXY,
    };

    /// Inserts one row per value into `encrypted.encrypted_text`.
    async fn insert_text(values: &[&str]) {
        for value in values {
            let id = random_id();
            execute_query(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &value.to_string()],
            )
            .await;
        }
    }

    /// Rows come back in plaintext order, ascending and descending, in both the
    /// extended and the simple protocol.
    ///
    /// The values are inserted out of order so that a passthrough of the raw
    /// jsonb — which sorts on the randomised ciphertext — could not produce the
    /// expected order by luck.
    #[tokio::test]
    async fn distinct_order_by_encrypted_text_is_ordered() {
        trace();
        clear().await;

        insert_text(&["cherry", "apple", "date", "banana"]).await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT DISTINCT encrypted_text FROM encrypted ORDER BY encrypted_text ASC";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["apple", "banana", "cherry", "date"], actual);

        let actual = simple_query::<String>(sql).await;
        assert_eq!(vec!["apple", "banana", "cherry", "date"], actual);

        let sql = "SELECT DISTINCT encrypted_text FROM encrypted ORDER BY encrypted_text DESC";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["date", "cherry", "banana", "apple"], actual);

        let actual = simple_query::<String>(sql).await;
        assert_eq!(vec!["date", "cherry", "banana", "apple"], actual);
    }

    /// Equal plaintexts collapse to one row.
    ///
    /// Encryption is randomised, so duplicates of the same plaintext hold
    /// different ciphertexts. Deduplicating on the payload would compare those
    /// ciphertexts and keep every row; the mapper keys on the equality term
    /// instead, which is equal exactly when the plaintexts are.
    #[tokio::test]
    async fn distinct_deduplicates_equal_plaintexts() {
        trace();
        clear().await;

        // Six rows, three distinct plaintexts.
        insert_text(&["cherry", "apple", "banana", "apple", "cherry", "apple"]).await;

        let client = connect_with_tls(*PROXY).await;

        // Without ORDER BY: deduplicated in place, no subquery wrapping.
        let sql = "SELECT DISTINCT encrypted_text FROM encrypted";
        let rows = client.query(sql, &[]).await.unwrap();
        let mut actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        actual.sort();
        assert_eq!(vec!["apple", "banana", "cherry"], actual);

        // With ORDER BY: deduplicated inside the wrapping subquery, ordered
        // outside it.
        let sql = "SELECT DISTINCT encrypted_text FROM encrypted ORDER BY encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["apple", "banana", "cherry"], actual);

        let actual = simple_query::<String>(sql).await;
        assert_eq!(vec!["apple", "banana", "cherry"], actual);
    }

    /// The ordering term the subquery projects must not reach the client: the
    /// result has exactly the columns that were asked for, under the names that
    /// were asked for.
    #[tokio::test]
    async fn distinct_order_by_does_not_leak_the_ordering_term() {
        trace();
        clear().await;

        insert_text(&["cherry", "apple"]).await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT DISTINCT id, encrypted_text FROM encrypted ORDER BY encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        assert_eq!(2, rows.len());

        let names: Vec<&str> = rows[0].columns().iter().map(|c| c.name()).collect();
        assert_eq!(vec!["id", "encrypted_text"], names);

        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["apple", "cherry"], actual);
    }

    /// An explicit alias survives the round trip through the subquery.
    #[tokio::test]
    async fn distinct_order_by_preserves_column_aliases() {
        trace();
        clear().await;

        insert_text(&["cherry", "apple"]).await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT DISTINCT encrypted_text AS fruit FROM encrypted ORDER BY encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let names: Vec<&str> = rows[0].columns().iter().map(|c| c.name()).collect();
        assert_eq!(vec!["fruit"], names);

        let actual: Vec<String> = rows.iter().map(|r| r.get("fruit")).collect();
        assert_eq!(vec!["apple", "cherry"], actual);
    }

    /// A plaintext column ordered alongside an encrypted one: the plaintext term
    /// is carried through as a reference to the column the subquery projects,
    /// and both sort keys still apply in order.
    #[tokio::test]
    async fn distinct_order_by_mixes_plaintext_and_encrypted_terms() {
        trace();
        clear().await;

        // Two groups, so the leading plaintext key decides and the encrypted key
        // breaks the tie within each group.
        for (plaintext, encrypted) in [
            ("b", "cherry"),
            ("a", "date"),
            ("b", "apple"),
            ("a", "banana"),
        ] {
            let id = random_id();
            execute_query(
                "INSERT INTO encrypted (id, plaintext, encrypted_text) VALUES ($1, $2, $3)",
                &[&id, &plaintext.to_string(), &encrypted.to_string()],
            )
            .await;
        }

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT DISTINCT plaintext, encrypted_text FROM encrypted \
                   ORDER BY plaintext, encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual: Vec<(String, String)> = rows
            .iter()
            .map(|r| (r.get("plaintext"), r.get("encrypted_text")))
            .collect();

        assert_eq!(
            vec![
                ("a".to_string(), "banana".to_string()),
                ("a".to_string(), "date".to_string()),
                ("b".to_string(), "apple".to_string()),
                ("b".to_string(), "cherry".to_string()),
            ],
            actual
        );
    }

    /// Ordering by an ordinal still refers to the right column: the wrapping
    /// projection preserves both the order and the count of the columns.
    #[tokio::test]
    async fn distinct_order_by_ordinal_alongside_encrypted_term() {
        trace();
        clear().await;

        for (plaintext, encrypted) in [("b", "cherry"), ("a", "date"), ("a", "banana")] {
            let id = random_id();
            execute_query(
                "INSERT INTO encrypted (id, plaintext, encrypted_text) VALUES ($1, $2, $3)",
                &[&id, &plaintext.to_string(), &encrypted.to_string()],
            )
            .await;
        }

        let client = connect_with_tls(*PROXY).await;

        // `1` is `plaintext`.
        let sql = "SELECT DISTINCT plaintext, encrypted_text FROM encrypted \
                   ORDER BY 1, encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual: Vec<(String, String)> = rows
            .iter()
            .map(|r| (r.get("plaintext"), r.get("encrypted_text")))
            .collect();

        assert_eq!(
            vec![
                ("a".to_string(), "banana".to_string()),
                ("a".to_string(), "date".to_string()),
                ("b".to_string(), "cherry".to_string()),
            ],
            actual
        );
    }

    /// `LIMIT` applies to the ordered result, not to some arbitrary prefix: it
    /// stays on the wrapping query rather than moving into the subquery.
    #[tokio::test]
    async fn distinct_order_by_applies_limit_after_ordering() {
        trace();
        clear().await;

        insert_text(&["cherry", "apple", "date", "banana"]).await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT DISTINCT encrypted_text FROM encrypted \
                   ORDER BY encrypted_text ASC LIMIT 2";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["apple", "banana"], actual);

        let sql = "SELECT DISTINCT encrypted_text FROM encrypted \
                   ORDER BY encrypted_text DESC LIMIT 2";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["date", "cherry"], actual);
    }
}
