//! EQL v3 variant of `select/indexing.rs`.
//!
//! v2 indexes the encrypted column with an on-column operator class:
//!
//! ```sql
//! CREATE INDEX ON encrypted (encrypted_text eql_v2.encrypted_operator_class);
//! ```
//!
//! v3 has no on-column operator class. Ordering/range queries engage
//! functional btree indexes on the term extractors instead:
//!
//! ```sql
//! CREATE INDEX ON encrypted (eql_v3.ord_term(encrypted_text));
//! CREATE INDEX ON encrypted (eql_v3.ord_ope_term(encrypted_int4));
//! ```

#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, insert, query_by, random_id, trace, PROXY};
    use tracing::info;

    ///
    /// Tests a range query with a functional index on the ORE term extractor.
    ///
    #[tokio::test]
    #[ignore = "blocked on eql-mapper v3"]
    async fn select_with_functional_index() {
        trace();

        for n in 1..=10 {
            let id = random_id();

            let encrypted_text = format!("hello_{}", n);

            let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
            insert(sql, &[&id, &encrypted_text]).await;
        }

        let client = connect_with_tls(PROXY).await;

        let sql = "CREATE INDEX IF NOT EXISTS encrypted_text_ord_term_idx ON encrypted (eql_v3.ord_term(encrypted_text))";
        client.simple_query(sql).await.unwrap();

        let sql = "EXPLAIN ANALYZE SELECT encrypted_text FROM encrypted WHERE encrypted_text <= $1";

        let encrypted_text = "hello_10".to_string();
        let result = query_by::<String>(sql, &encrypted_text).await;

        info!("Result: {:?}", result);

        // EXPLAIN ANALYZE returns the executed plan as rows of text
        assert!(!result.is_empty());
    }
}
