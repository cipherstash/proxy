//! One placeholder bound in two roles at once.
//!
//! `UPDATE t SET enc = $1 WHERE enc = $1` binds the same input param as both a
//! stored value and a query operand. The two need different payloads — the
//! stored one carries the ciphertext, the query one carries only search terms —
//! so the role has to be tracked per occurrence in the rewritten statement.
//!
//! Tracking it per *input* param instead marks both occurrences as query
//! operands, which strips the ciphertext from the `SET` value and fails its cast
//! to the column's own domain:
//!
//! ```text
//! ERROR: value for domain eql_v3_text_search violates check constraint "eql_v3_text_search_check"
//! ```

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, execute_query, query, random_id, trace, PROXY};

    #[tokio::test]
    async fn update_with_param_reused_for_storage_and_query() {
        trace();
        clear().await;

        let id = random_id();
        let original = "hello@cipherstash.com".to_string();
        execute_query(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
            &[&id, &original],
        )
        .await;

        let client = connect_with_tls(PROXY).await;

        // The same placeholder is the stored value and the predicate operand.
        let sql = "UPDATE encrypted SET encrypted_text = $1 WHERE encrypted_text = $1";
        client
            .execute(sql, &[&original])
            .await
            .expect("a param bound as both a stored value and a query operand should work");

        // The row is still there, still decryptable, still itself.
        let actual = query::<String>("SELECT encrypted_text FROM encrypted").await;
        assert_eq!(vec![original.clone()], actual);
    }

    /// The same shape, but the stored value differs from the one searched for —
    /// so the update has to actually take effect, not merely be accepted.
    #[tokio::test]
    async fn update_rewrites_the_row_matched_by_the_reused_param() {
        trace();
        clear().await;

        let id = random_id();
        let original = "hello@cipherstash.com".to_string();
        execute_query(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
            &[&id, &original],
        )
        .await;

        let client = connect_with_tls(PROXY).await;

        // `$1` stores, `$2` queries; the reverse of the pairing above.
        let updated = "goodbye@cipherstash.com".to_string();
        let sql = "UPDATE encrypted SET encrypted_text = $1 WHERE encrypted_text = $2";
        let n = client.execute(sql, &[&updated, &original]).await.unwrap();
        assert_eq!(1, n, "the WHERE operand should have matched the stored row");

        let actual = query::<String>("SELECT encrypted_text FROM encrypted").await;
        assert_eq!(vec![updated], actual);
    }
}
