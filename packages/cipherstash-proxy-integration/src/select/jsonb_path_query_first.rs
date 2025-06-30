#[cfg(test)]
mod tests {
    use crate::common::{
        clear, insert_jsonb, query_by, simple_query, simple_query_with_null, trace,
    };
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    async fn select_jsonb<T>(selector: &str, value: T)
    where
        T: DeserializeOwned,
        serde_json::Value: From<T>,
    {
        let selector = JsonPath::new(selector);
        let value = Value::from(value);

        let expected = vec![value];

        let sql = "SELECT jsonb_path_query_first(encrypted_jsonb, $1) FROM encrypted";
        let actual = query_by::<Value>(sql, &selector).await;

        assert_expected(&expected, &actual);

        let sql =
            format!("SELECT jsonb_path_query_first(encrypted_jsonb, '{selector}') FROM encrypted");
        let actual = simple_query::<Value>(&sql).await;

        assert_expected(&expected, &actual);
    }

    #[tokio::test]
    async fn select_jsonb_path_query_first_string() {
        trace();

        clear().await;

        insert_jsonb().await;

        select_jsonb("$.array_string[*]", "hello".to_string()).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_query_first_number() {
        trace();

        clear().await;

        insert_jsonb().await;

        select_jsonb("$.array_number[*]", 42).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_query_first_with_unknown() {
        trace();

        clear().await;

        insert_jsonb().await;

        let selector = JsonPath::new("$.vtha");

        let sql = "SELECT jsonb_path_query_first(encrypted_jsonb, $1) as selected FROM encrypted";
        let actual = query_by::<Option<Value>>(sql, &selector).await;

        let expected = vec![None];
        assert_expected(&expected, &actual);

        let sql = format!(
            "SELECT jsonb_path_query_first(encrypted_jsonb, '{selector}') as selected FROM encrypted"
        );

        // Expect again for Vec<Option<String>> type
        let expected = vec![None];
        let actual = simple_query_with_null(&sql).await;

        assert_expected(&expected, &actual);
    }

    // #[tokio::test]
    // async fn select_jsonb_path_query_first_string() {
    //     trace();

    //     clear().await;

    //     insert_jsonb().await;
    //     insert_jsonb().await;

    //     select_jsonb("$.nested.string", "world".to_string()).await;
    // }

    // #[tokio::test]
    // async fn select_jsonb_path_query_first_value() {
    //     trace();

    //     clear().await;

    //     insert_jsonb().await;
    //     insert_jsonb().await;

    //     let v = serde_json::json!({
    //         "number": 1815,
    //         "string": "world",
    //     });

    //     select_jsonb("$.nested", v).await;
    // }

    // #[tokio::test]
    // async fn select_jsonb_path_query_first_with_alias() {
    //     trace();

    //     clear().await;

    //     insert_jsonb().await;
    //     insert_jsonb().await;

    //     let value = serde_json::json!({
    //         "number": 1815,
    //         "string": "world",
    //     });

    //     let selector = JsonPath::new("$.nested");

    //     let expected = vec![value];

    //     let sql = "SELECT jsonb_path_query_first(encrypted_jsonb, $1) as selected FROM encrypted";
    //     let actual = query_by::<Value>(sql, &selector).await;

    //     assert_expected(&expected, &actual);

    //     let sql = format!(
    //         "SELECT jsonb_path_query_first(encrypted_jsonb, '{selector}') as selected FROM encrypted"
    //     );
    //     let actual = simple_query::<Value>(&sql).await;

    //     assert_expected(&expected, &actual);
    // }
}
