#[cfg(test)]
mod tests {
    use crate::common::{clear, insert_jsonb, query_by, simple_query, trace};
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde_json::Value;

    async fn select_jsonb(selector: &str, expected: &[Value]) {
        let selector = JsonPath::new(selector);

        let sql =
            "SELECT jsonb_array_elements(jsonb_path_query(encrypted_jsonb, $1)) FROM encrypted";
        let actual = query_by::<Value>(sql, &selector).await;

        assert_expected(expected, &actual);

        let sql = format!("SELECT jsonb_array_elements(jsonb_path_query(encrypted_jsonb, '{selector}')) FROM encrypted");
        let actual = simple_query::<Value>(&sql).await;

        assert_expected(expected, &actual);
    }

    #[tokio::test]
    async fn select_jsonb_array_elements_with_string() {
        trace();

        clear().await;
        insert_jsonb().await;

        let expected = vec![Value::from("hello"), Value::from("world")];
        select_jsonb("$.array_string[@]", &expected).await;
    }

    #[tokio::test]
    async fn select_jsonb_array_elements_with_numeric() {
        trace();

        clear().await;

        insert_jsonb().await;

        let expected = vec![Value::from(42), Value::from(84)];
        select_jsonb("$.array_number[@]", &expected).await;
    }

    #[tokio::test]
    async fn select_jsonb_array_elements_with_unknown_field() {
        trace();

        clear().await;
        insert_jsonb().await;

        let expected = vec![];
        select_jsonb("$.blah", &expected).await;
    }
}
