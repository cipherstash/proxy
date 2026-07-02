//! EQL v3 replacement surface for the v2 match-index tests.
//!
//! v2's `map_match_index.rs` exercises `LIKE` over the match index. EQL v3
//! does NOT support `LIKE`/`ILIKE`: the match index (bloom filter, `bf` term)
//! only supports containment, via `@>` / `<@` (`eql_v3.contains`). There is
//! deliberately no v3 port of the LIKE/ILIKE tests; this placeholder documents
//! and exercises the replacement operator surface.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};

    /// v2: `WHERE encrypted_text LIKE $1` (match index).
    /// v3: `WHERE encrypted_text @> $1` - bloom containment on the `bf` term
    /// of `eql_v3.text_search` / `eql_v3.text_match`.
    #[tokio::test]
    #[ignore = "blocked on eql-mapper v3"]
    async fn match_index_bloom_containment_replaces_like() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE encrypted_text @> $1";
        let rows = client.query(sql, &[&"hello@"]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }
    }
}
