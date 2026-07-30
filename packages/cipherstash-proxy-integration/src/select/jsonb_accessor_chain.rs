//! Multi-step encrypted JSON accessor chains, everywhere — not only under an
//! exact equality.
//!
//! An encrypted JSON column is a SteVec document: an `sv` array of entries, each
//! keyed by a selector MAC. `eql_v3."->"(doc, sel)` searches that array, and what
//! it returns is one ENTRY, which has no `sv` of its own. So a chain cannot be two
//! hops — the outer call would search an entry, find nothing, and return NULL.
//!
//! The correct emission is ONE accessor carrying the composed path:
//! `j -> 'a' -> 'b'` becomes `eql_v3."->"(j, <selector for $.a.b>)`. These tests
//! read a nested field back through Proxy and assert the **value**, because the
//! failure being guarded against is not an error — it is a query that runs
//! perfectly and answers NULL.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, execute_query, random_id, trace, PROXY};
    use serde_json::Value;

    /// A document with a field two levels down, and a number there too so that
    /// ordering has something to compare.
    async fn insert_nested() -> i64 {
        let id = random_id();
        let doc = serde_json::json!({
            "nested": { "string": "world", "number": 42 },
            "string": "hello",
        });

        execute_query(
            "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
            &[&id, &doc],
        )
        .await;

        id
    }

    /// The single value `sql` projects, decrypted by Proxy.
    async fn project(sql: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> Value {
        let client = connect_with_tls(*PROXY).await;

        let rows = client
            .query(sql, params)
            .await
            .unwrap_or_else(|e| panic!("`{sql}` should execute, got: {e}"));

        assert_eq!(rows.len(), 1, "expected exactly one row from `{sql}`");

        rows[0].get(0)
    }

    /// Projecting a two-level field returns the field, not NULL.
    ///
    /// This is the whole point. The chain used to be refused at type check, which
    /// was better than the alternative it replaced — emitting a second
    /// entry-scoped accessor over an entry and answering NULL with no error at
    /// all. Now it is composed into one path and actually works.
    #[tokio::test]
    async fn two_level_projection_returns_the_field() {
        trace();
        clear().await;
        insert_nested().await;

        assert_eq!(
            project(
                "SELECT encrypted_jsonb -> 'nested' -> 'string' FROM encrypted",
                &[]
            )
            .await,
            Value::String("world".to_string())
        );
    }

    /// Every spelling of the same path reads the same field.
    ///
    /// Brackets carry no meaning, `->>` differs from `->` only in the result type,
    /// and `jsonb_path_query_first` is the function spelling of a step. All four
    /// decompose to the same root and the same composed path, so all four must
    /// return the same value — a walker that missed one would compose a short path
    /// and read a different field.
    #[tokio::test]
    async fn every_spelling_of_a_chain_reads_the_same_field() {
        trace();
        clear().await;
        insert_nested().await;

        for sql in [
            "SELECT encrypted_jsonb -> 'nested' -> 'string' FROM encrypted",
            "SELECT (encrypted_jsonb -> 'nested') -> 'string' FROM encrypted",
            "SELECT encrypted_jsonb -> 'nested' ->> 'string' FROM encrypted",
            "SELECT jsonb_path_query_first(encrypted_jsonb, '$.nested') -> 'string' \
             FROM encrypted",
        ] {
            assert_eq!(
                project(sql, &[]).await,
                Value::String("world".to_string()),
                "unexpected value for `{sql}`"
            );
        }
    }

    /// A chain whose outermost step is a placeholder resolves at Bind.
    ///
    /// The literal step is known at Parse time and the param step is not, so the
    /// composed path can only be built once the param is bound — which is why this
    /// needs a record carried through to the proxy rather than a rewrite alone.
    /// The surviving operand is the param, so the whole path resolves together.
    #[tokio::test]
    async fn a_param_step_in_a_projected_chain_resolves_at_bind() {
        trace();
        clear().await;
        insert_nested().await;

        assert_eq!(
            project(
                "SELECT encrypted_jsonb -> 'nested' -> $1 FROM encrypted",
                &[&"string"]
            )
            .await,
            Value::String("world".to_string())
        );

        // Both steps bound: the composed path is entirely a Bind-time value.
        assert_eq!(
            project(
                "SELECT encrypted_jsonb -> $1 -> $2 FROM encrypted",
                &[&"nested", &"string"]
            )
            .await,
            Value::String("world".to_string())
        );
    }

    /// A placeholder step in front of a LITERAL outermost selector is refused,
    /// not answered from a short path.
    ///
    /// The operand that survives the collapse is the outermost selector, and a
    /// literal is encrypted at Parse time — before any param is bound. So the path
    /// `$.<$1>.string` cannot be composed when it is needed. The mirror image
    /// (`-> 'nested' -> $1`) works, because there the surviving operand is the
    /// param and the whole path resolves together at Bind.
    ///
    /// This is the same limitation the fused equality has for `col -> $1 = 'value'`
    /// and for the same reason. Composing only what is known would key `$.string`
    /// and read the wrong field, silently — so refusing is the only safe answer.
    #[tokio::test]
    async fn a_param_step_before_a_literal_selector_is_refused() {
        trace();
        clear().await;
        insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let result = client
            .query(
                "SELECT encrypted_jsonb -> $1 -> 'string' FROM encrypted",
                &[&"nested"],
            )
            .await;

        match result {
            Err(_) => {}
            Ok(rows) => {
                // If it is not refused it must at least not have answered from a
                // truncated path: `$.string` holds "hello", which is the wrong
                // field and the failure this asserts against.
                for row in rows {
                    let value: Option<Value> = row.get(0);
                    assert_ne!(
                        value,
                        Some(Value::String("hello".to_string())),
                        "a truncated path answered the wrong field"
                    );
                }
            }
        }
    }

    /// A chain keys the WHOLE path, so a prefix of it selects nothing.
    ///
    /// `$.string` holds "hello" and `$.nested.string` holds "world". If the
    /// composition dropped the inner step the chain would read `$.string` and
    /// answer "hello" — a silently wrong answer rather than an error, which is
    /// exactly the failure mode this guards.
    #[tokio::test]
    async fn a_chain_reads_the_composed_path_not_a_prefix_of_it() {
        trace();
        clear().await;
        insert_nested().await;

        let nested = project(
            "SELECT encrypted_jsonb -> 'nested' -> 'string' FROM encrypted",
            &[],
        )
        .await;
        let top = project("SELECT encrypted_jsonb -> 'string' FROM encrypted", &[]).await;

        assert_eq!(nested, Value::String("world".to_string()));
        assert_eq!(top, Value::String("hello".to_string()));
        assert_ne!(
            nested, top,
            "a chain must not collapse to its outermost step alone"
        );
    }

    /// A path the document does not have selects nothing, and says so as NULL.
    #[tokio::test]
    async fn a_chain_selecting_a_missing_path_is_null() {
        trace();
        clear().await;
        insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let rows = client
            .query(
                "SELECT encrypted_jsonb -> 'nested' -> 'absent' FROM encrypted",
                &[],
            )
            .await
            .unwrap();

        let value: Option<Value> = rows[0].get(0);
        assert_eq!(value, None);
    }

    /// Ordering over a two-level path compares the field at that path.
    ///
    /// The accessor SURVIVES here — the comparison wraps it in `eql_v3.ord_term`
    /// rather than absorbing it the way equality does — so this exercises the
    /// collapsed accessor in a predicate rather than a projection. A chain that
    /// stayed two hops would compare NULL and match nothing at all, which reads as
    /// "no rows" rather than as a failure.
    #[tokio::test]
    async fn ordering_on_a_two_level_path_compares_that_field() {
        trace();
        clear().await;
        let id = insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        // `$.nested.number` is 42.
        for (sql, expected) in [
            (
                "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> 'number' < $1",
                vec![id],
            ),
            (
                "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> 'number' >= $1",
                vec![],
            ),
        ] {
            let rows = client
                .query(sql, &[&Value::from(100)])
                .await
                .unwrap_or_else(|e| panic!("`{sql}` should execute, got: {e}"));

            let actual: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
            assert_eq!(actual, expected, "unexpected rows for `{sql}`");
        }

        // The boundary, to show the comparison is against 42 and not against
        // whatever a NULL comparison would yield.
        let rows = client
            .query(
                "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> 'number' >= $1",
                &[&Value::from(42)],
            )
            .await
            .unwrap();

        let actual: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
        assert_eq!(actual, vec![id], "42 >= 42 must match");
    }

    /// Equality over a chain must keep FUSING, not degrade to an accessor plus a
    /// comparison.
    ///
    /// The fused needle MACs the path and the value together and its presence in
    /// the stored `sv` is the match — strictly stronger than extracting a field and
    /// then comparing it. This is the shape that already worked; it must go on
    /// working unchanged now that chains collapse everywhere else.
    #[tokio::test]
    async fn equality_over_a_chain_still_matches() {
        trace();
        clear().await;
        let id = insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let rows = client
            .query(
                "SELECT id FROM encrypted WHERE encrypted_jsonb -> 'nested' -> 'string' = $1",
                &[&Value::String("world".to_string())],
            )
            .await
            .unwrap();

        let actual: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
        assert_eq!(actual, vec![id]);
    }

    /// A chain split across a subquery boundary stays refused.
    ///
    /// This one is impossible, not unimplemented. `EqlTerm::JsonExtracted` does not
    /// carry the path that produced it, and the root column is not even in scope in
    /// the outer query — so there is nothing to root a composed path at without
    /// rewriting the subquery's projection.
    ///
    /// What the client sees depends on `mapping_errors_enabled`; what is pinned
    /// here is that Proxy never rewrites it into an accessor over an entry. The
    /// type-check refusal itself is pinned by
    /// `eql_mapper::test::json_operation_on_an_extracted_value_is_refused`.
    #[tokio::test]
    async fn a_chain_split_across_a_subquery_is_not_rewritten() {
        trace();
        clear().await;
        insert_nested().await;

        let client = connect_with_tls(*PROXY).await;

        let result = client
            .query(
                "SELECT a -> 'foo' FROM \
                 (SELECT encrypted_jsonb -> 'nested' AS a FROM encrypted) s",
                &[],
            )
            .await;

        // Refused outright, or passed through unmapped — but never answered with a
        // value, which would mean a path was composed that cannot be.
        if let Ok(rows) = result {
            for row in rows {
                let value: Option<Value> = row.get(0);
                assert_eq!(
                    value, None,
                    "a chain rooted at an extracted entry must not resolve to a value"
                );
            }
        }
    }
}
