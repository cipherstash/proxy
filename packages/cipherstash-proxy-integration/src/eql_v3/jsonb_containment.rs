//! EQL v3 variant of the explicit-function test in
//! `select/jsonb_containment_index.rs`.
//!
//! v2 exposes SteVec containment both as the `@>` operator and as an explicit
//! `eql_v2.jsonb_contains()` function. v3 has no `jsonb_contains` function:
//! containment is expressed only through the `@>` / `<@` operators on
//! `eql_v3.json`, with `eql_v3.jsonb_query` needles (see
//! `eql_v3.to_ste_vec_query` for the functional GIN index expression).
//!
//! The operator-shaped containment tests in `select/jsonb_containment_index.rs`
//! ride on the mapper and the fixture, and are not duplicated here.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};
    use serde_json::json;

    /// v2: `WHERE eql_v2.jsonb_contains(encrypted_jsonb, $1)`.
    /// v3: `WHERE encrypted_jsonb @> $1` - there is no function-call form.
    #[tokio::test]
    #[ignore = "blocked on eql-mapper v3"]
    async fn jsonb_containment_operator_replaces_jsonb_contains_function() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        // 20 rows of {"string": "value_N", "number": n} with N = n % 10,
        // giving exactly 2 rows per "value_N".
        for n in 1..=20_i64 {
            let id = random_id();
            let encrypted_jsonb = json!({
                "string": format!("value_{}", n % 10),
                "number": n,
            });

            let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
            client.query(sql, &[&id, &encrypted_jsonb]).await.unwrap();
        }

        let search_value = json!({"string": "value_1"});
        let sql = "SELECT COUNT(*) FROM encrypted WHERE encrypted_jsonb @> $1";

        let rows = client.query(sql, &[&search_value]).await.unwrap();
        let count: i64 = rows[0].get(0);

        assert_eq!(count, 2);
    }
}
