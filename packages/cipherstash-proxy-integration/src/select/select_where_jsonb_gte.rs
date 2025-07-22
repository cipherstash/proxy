#[cfg(test)]
mod tests {
    use crate::common::{clear, insert_jsonb_for_search, query_by_params, simple_query, trace};
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde_json::Value;

    async fn select_where_jsonb(selector: &str, value: Value, expected: &[Value]) {
        clear().await;
        insert_jsonb_for_search().await;

        let json_path_selector = JsonPath::new(selector);

        // WHERE -> with extended
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 >= $2";
        let actual = query_by_params::<Value>(sql, &[&selector, &value]).await;
        assert_expected(expected, &actual);

        // WHERE -> with simple
        let sql =
            format!(
                "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> '{selector}' >= '{value}'"
        );
        let actual = simple_query::<Value>(&sql).await;
        assert_expected(expected, &actual);

        // WHERE jsonb_path_query_first with extended
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, $1) >= $2";
        let actual = query_by_params::<Value>(sql, &[&json_path_selector, &value]).await;
        assert_expected(expected, &actual);

        // WHERE jsonb_path_query_first with simple
        let sql = format!(
            "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, '{selector}') >= '{value}'"
        );
        let actual = simple_query::<Value>(&sql).await;
        assert_expected(expected, &actual);
    }

    #[tokio::test]
    async fn select_jsonb_where_string_gte() {
        trace();

        let selector = "string";
        let value = Value::from("C");

        let expected = vec![
            serde_json::json!({
                "string": "C",
                "number": 3,
            }),
            serde_json::json!({
                "string": "D",
                "number": 4,
            }),
            serde_json::json!({
                "string": "E",
                "number": 5,
            }),
        ];

        select_where_jsonb(selector, value, &expected).await;
    }

    #[tokio::test]
    async fn select_jsonb_where_numeric_gte() {
        trace();

        let selector = "number";
        let value = Value::from(4);

        let expected = vec![
            serde_json::json!({
                "string": "D",
                "number": 4,
            }),
            serde_json::json!({
                "string": "E",
                "number": 5,
            }),
        ];

        select_where_jsonb(selector, value, &expected).await;
    }
}
