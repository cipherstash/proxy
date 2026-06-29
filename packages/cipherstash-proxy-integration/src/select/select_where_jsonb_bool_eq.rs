#[cfg(test)]
mod tests {
    use crate::common::{
        assert_encrypted_jsonb, clear, insert, query_by_params, random_id, simple_query, trace,
    };
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde_json::Value;

    /// Inserts two documents that differ only in a boolean `flag` leaf, so a
    /// `-> 'flag' = <bool>` equality predicate must select exactly one of them.
    ///
    /// A boolean sv leaf carries the `hm` (HMAC) equality term, so equality is
    /// supported end-to-end (ordering is not — booleans have no `oc` CLLW ORE
    /// term). This guards the `Value::Bool` arm of `json_scalar_to_plaintext`,
    /// which reduces the boolean query term to a scalar `Plaintext::Boolean`.
    async fn insert_jsonb_with_bool() {
        for flag in [true, false] {
            let id = random_id();
            let doc = serde_json::json!({
                "string": flag.to_string(),
                "flag": flag,
            });

            let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
            insert(sql, &[&id, &doc]).await;
            assert_encrypted_jsonb(id, &doc).await;
        }
    }

    async fn select_where_jsonb_flag(value: bool, expected: &[Value]) {
        clear().await;
        insert_jsonb_with_bool().await;

        let selector = "flag";
        let json_path_selector = JsonPath::new(selector);

        // WHERE -> with extended
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 = $2";
        let actual = query_by_params::<Value>(sql, &[&selector, &value]).await;
        assert_expected(expected, &actual);

        // WHERE -> with simple
        let sql = format!(
            "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> '{selector}' = {value}"
        );
        let actual = simple_query::<Value>(&sql).await;
        assert_expected(expected, &actual);

        // WHERE jsonb_path_query_first with extended
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, $1) = $2";
        let actual = query_by_params::<Value>(sql, &[&json_path_selector, &value]).await;
        assert_expected(expected, &actual);

        // WHERE jsonb_path_query_first with simple
        let sql = format!(
            "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, '{selector}') = {value}"
        );
        let actual = simple_query::<Value>(&sql).await;
        assert_expected(expected, &actual);
    }

    #[tokio::test]
    async fn select_jsonb_where_bool_true_eq() {
        trace();

        let expected = vec![serde_json::json!({
            "string": "true",
            "flag": true,
        })];

        select_where_jsonb_flag(true, &expected).await;
    }

    #[tokio::test]
    async fn select_jsonb_where_bool_false_eq() {
        trace();

        let expected = vec![serde_json::json!({
            "string": "false",
            "flag": false,
        })];

        select_where_jsonb_flag(false, &expected).await;
    }
}
