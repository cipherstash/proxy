//! Shapes where JSON value-selector fusion sends plaintext to the database.
//!
//! `col -> 'field' = value` is rewritten by fusing the field and the value into
//! a single encrypted needle matched by containment, so neither half is ever
//! visible on its own. The two shapes below reach that rewrite by routes it does
//! not handle, and in each case something the client wrote in plaintext is
//! forwarded to PostgreSQL.
//!
//! # These tests are ignored, not deleted
//!
//! Each asserts the behaviour the shape must have. They fail today; un-ignoring
//! one is the acceptance test for its fix. Rejecting the shape at type-check
//! time is an equally acceptable outcome — a clear error is not a leak — in
//! which case the test should be rewritten to assert the error.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, execute_query, random_id, trace, PROXY};
    use serde_json::Value;

    async fn insert_nested() -> i64 {
        let id = random_id();
        let doc = serde_json::json!({
            "nested": { "string": "world" },
            "string": "hello",
        });

        execute_query(
            "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
            &[&id, &doc],
        )
        .await;

        id
    }

    /// A chained accessor must not put the intermediate selector in the SQL, nor
    /// run native `->` on the encrypted payload.
    ///
    /// Confirmed emitted SQL:
    ///
    /// ```text
    /// eql_v3.jsonb_contains(encrypted_jsonb -> 'nested', '{…}')
    /// ```
    ///
    /// The container is cloned from the *original* AST, so the inner
    /// `-> 'nested'` survives untouched: the plaintext field name `'nested'`
    /// ships in the statement text, and native jsonb `->` is applied to the
    /// encrypted payload — which also makes the predicate match nothing.
    #[tokio::test]
    #[ignore = "Chained JSON accessor (col -> 'a' -> 'b' = value) clones the container from the \
                original AST, leaking the plaintext selector 'a' into the SQL text and running \
                native jsonb -> on the encrypted payload. Returns 0 rows as well as leaking. See \
                rewrite_json_value_selector_eq.rs."]
    async fn chained_accessor_does_not_leak_the_selector() {
        trace();
        clear().await;
        let id = insert_nested().await;

        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> 'string' = $1";
        let rows = client
            .query(sql, &[&Value::String("world".to_string())])
            .await
            .expect("a chained accessor comparison should be supported or rejected, not mis-sent");

        let actual: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
        assert_eq!(vec![id], actual);
    }

    /// A NULL selector must not forward the *value* operand in plaintext.
    ///
    /// With the path bound NULL there is no needle to build, so encryption is
    /// skipped — but the rebuild path forwards the value's raw client bytes, so
    /// the comparand crosses the wire and can land in the server log when the
    /// domain CHECK rejects it. Confirmed: PostgreSQL received the plaintext and
    /// failed with `cannot call jsonb_each on a non-object`.
    ///
    /// `col -> NULL = x` is NULL in SQL, so the correct result is simply no
    /// rows.
    #[tokio::test]
    #[ignore = "A NULL selector param forwards the VALUE operand to the database in plaintext: \
                json_value_selector_plaintext yields nothing, encryption is skipped, and \
                bind.rs's rebuild path passes the client's raw bytes through. Should bind NULL \
                and return no rows."]
    async fn null_selector_param_does_not_forward_plaintext() {
        trace();
        clear().await;
        insert_nested().await;

        let client = connect_with_tls(PROXY).await;

        let selector: Option<String> = None;
        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb -> $1 = $2";

        let rows = client
            .query(sql, &[&selector, &Value::String("world".to_string())])
            .await
            .expect("a NULL selector should compare as NULL, not send the value in plaintext");

        assert!(
            rows.is_empty(),
            "col -> NULL = x is NULL in SQL, so no rows should match"
        );
    }
}
