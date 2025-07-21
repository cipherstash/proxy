#[cfg(test)]
mod tests {
    use crate::common::{clear, insert, query_by_params, random_id, simple_query, trace};
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use tracing::info;

    async fn select_where_jsonb(selector: &str, value: Value, expected: &[Value]) {
        let json_path_selector = JsonPath::new(selector);
        let value = Value::from(value);

        // WHERE -> with extended
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 >= $2";
        let actual = query_by_params::<Value>(sql, &[&selector, &value]).await;
        assert_expected(&expected, &actual);

        // WHERE -> with simple
        let sql =
            format!(
                "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> '{selector}' >= '{value}'"
        );
        let actual = simple_query::<Value>(&sql).await;
        assert_expected(&expected, &actual);

        // WHERE jsonb_path_query_first with extended
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, $1) >= $2";
        let actual = query_by_params::<Value>(sql, &[&json_path_selector, &value]).await;
        assert_expected(&expected, &actual);

        // WHERE jsonb_path_query_first with simple
        let sql = format!(
            "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, '{selector}') >= '{value}'"
        );
        let actual = simple_query::<Value>(&sql).await;
        assert_expected(&expected, &actual);
    }

    pub async fn insert_jsonb() {
        for n in 1..=5 {
            let id = random_id();
            let s = ((b'A' + (n - 1) as u8) as char).to_string();

            let encrypted_jsonb = serde_json::json!({
                "string": s,
                "number": n,
            });

            let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
            insert(sql, &[&id, &encrypted_jsonb]).await;
        }
    }

    #[tokio::test]
    async fn select_jsonb_where_string_gte() {
        trace();

        clear().await;
        insert_jsonb().await;

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

        clear().await;
        insert_jsonb().await;

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
