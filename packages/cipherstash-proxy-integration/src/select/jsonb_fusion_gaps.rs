//! Shapes where JSON value-selector fusion used to send plaintext to the
//! database.
//!
//! `col -> 'field' = value` is rewritten by fusing the field and the value into
//! a single encrypted needle matched by containment, so neither half is ever
//! visible on its own. The two shapes below reach that rewrite by routes it did
//! not handle, and in each case something the client wrote in plaintext was
//! forwarded to PostgreSQL (CIP-3682).
//!
//! Both are fixed. A chained accessor is now one path (`$.nested.string`)
//! rooted at the bare column, and a fusion that cannot build a needle binds
//! NULL instead of the client's bytes. The tests assert the behaviour AND, by
//! reading the stored row on a direct connection that bypasses Proxy, that
//! nothing the client wrote in the clear is visible to the database.

#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, execute_query, get_database_port, random_id, trace, PROXY,
    };
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

    /// The stored payload, read on a connection straight to PostgreSQL so that
    /// Proxy never gets to decrypt it. This is what the database actually holds.
    ///
    /// The port comes from `get_database_port()` — the database Proxy is backed
    /// by — not from `PG_PORT`. Under the TLS suite Proxy sits in front of
    /// `postgres-tls` on 5617 while `PG_PORT` is 5532, so a hardcoded `PG_PORT`
    /// reads a different, empty database and finds no row at all.
    async fn stored_payload(id: i64) -> String {
        let client = connect_with_tls(get_database_port()).await;

        let rows = client
            .query(
                "SELECT encrypted_jsonb::text AS payload FROM encrypted WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap();

        rows[0].get("payload")
    }

    /// A chained accessor selects one path of one document, and must not put any
    /// step of that path in the SQL, nor run native `->` on the encrypted
    /// payload.
    ///
    /// It used to emit `eql_v3.jsonb_contains(encrypted_jsonb -> 'nested', …)`:
    /// the container was cloned from the original AST, so the inner
    /// `-> 'nested'` survived untouched. The plaintext field name shipped in the
    /// statement text, and native jsonb `->` was applied to the encrypted
    /// payload — which also made the predicate match nothing.
    #[tokio::test]
    async fn chained_accessor_does_not_leak_the_selector() {
        trace();
        clear().await;
        let id = insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> 'string' = $1";
        let rows = client
            .query(sql, &[&Value::String("world".to_string())])
            .await
            .expect("a chained accessor comparison should be supported or rejected, not mis-sent");

        let actual: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
        assert_eq!(vec![id], actual);

        // Nothing the client wrote is visible to the database: not the field
        // names it traversed, not the value it compared.
        let payload = stored_payload(id).await;
        for plaintext in ["nested", "string", "world", "hello"] {
            assert!(
                !payload.contains(plaintext),
                "the stored payload leaks `{plaintext}`: {payload}"
            );
        }
    }

    /// The same chain written with placeholder steps: every step is dropped from
    /// the statement and folded into the needle instead.
    #[tokio::test]
    async fn chained_accessor_with_param_selectors_matches() {
        trace();
        clear().await;
        let id = insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb -> $1 -> $2 = $3";
        let rows = client
            .query(
                sql,
                &[&"nested", &"string", &Value::String("world".to_string())],
            )
            .await
            .expect("a chained accessor with placeholder selectors should be supported");

        let actual: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
        assert_eq!(vec![id], actual);
    }

    /// A chain that selects a path the document does not have matches nothing —
    /// the needle is keyed on the whole path, so a wrong step cannot match a
    /// right one.
    #[tokio::test]
    async fn chained_accessor_with_a_wrong_path_matches_nothing() {
        trace();
        clear().await;
        insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        // `$.string` holds "hello" and `$.nested.string` holds "world"; neither
        // is `$.nested.hello`, and the value belongs to a different path.
        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> 'string' = $1";
        let rows = client
            .query(sql, &[&Value::String("hello".to_string())])
            .await
            .unwrap();

        assert!(
            rows.is_empty(),
            "a value stored at another path must not match this one"
        );
    }

    /// `<>` on a chain is the same containment, negated.
    #[tokio::test]
    async fn chained_accessor_not_eq_excludes_the_match() {
        trace();
        clear().await;
        let id = insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> 'string' <> $1";
        let rows = client
            .query(sql, &[&Value::String("world".to_string())])
            .await
            .unwrap();

        assert!(
            rows.is_empty(),
            "the row whose `$.nested.string` IS `world` must be excluded, got {} row(s)",
            rows.len()
        );

        let rows = client
            .query(sql, &[&Value::String("elsewhere".to_string())])
            .await
            .unwrap();

        let actual: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
        assert_eq!(vec![id], actual);
    }

    /// A NULL selector must not forward the *value* operand in plaintext.
    ///
    /// With the path bound NULL there is no needle to build, so encryption is
    /// skipped — and the rebuild path used to forward the value's raw client
    /// bytes, so the comparand crossed the wire and could land in the server log
    /// when the domain CHECK rejected it. Confirmed: PostgreSQL received the
    /// plaintext and failed with `cannot call jsonb_each on a non-object`.
    ///
    /// `col -> NULL = x` is NULL in SQL, so the correct result is simply no
    /// rows.
    #[tokio::test]
    async fn null_selector_param_does_not_forward_plaintext() {
        trace();
        clear().await;
        insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

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

    /// A NULL step anywhere in a chain is the same: no needle, no rows, and
    /// nothing of the client's forwarded.
    #[tokio::test]
    async fn null_step_in_a_chain_does_not_forward_plaintext() {
        trace();
        clear().await;
        insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let selector: Option<String> = None;
        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> $1 = $2";

        let rows = client
            .query(sql, &[&selector, &Value::String("world".to_string())])
            .await
            .expect("a NULL step should compare as NULL, not send the value in plaintext");

        assert!(rows.is_empty(), "a NULL step matches nothing");
    }

    /// A NULL *value* is the mirror case: nothing to encrypt, and nothing of the
    /// client's to forward.
    #[tokio::test]
    async fn null_value_param_does_not_forward_plaintext() {
        trace();
        clear().await;
        insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let value: Option<Value> = None;
        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb -> $1 = $2";

        let rows = client
            .query(sql, &[&"nested", &value])
            .await
            .expect("a NULL value should compare as NULL");

        assert!(rows.is_empty(), "col -> 'nested' = NULL matches nothing");
    }
}
