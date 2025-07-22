#[cfg(test)]
mod tests {
    use crate::{
        common::{clear, insert_jsonb, query_by, simple_query, simple_query_with_null, trace},
        support::assert::assert_expected,
    };
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    async fn select_get_jsonb_field<T>(selector: &str, value: T)
    where
        T: DeserializeOwned,
        serde_json::Value: From<T>,
    {
        let value = Value::from(value);
        let expected = vec![value];

        let sql = "SELECT encrypted_jsonb->$1 FROM encrypted LIMIT 1";
        let actual = query_by::<Value>(sql, &selector).await;

        assert_expected(&expected, &actual);

        let sql = format!("SELECT encrypted_jsonb  -> '{selector}' FROM encrypted LIMIT 1");
        let actual = simple_query::<Value>(&sql).await;

        assert_expected(&expected, &actual);
    }

    async fn select_get_jsonb_field_null(selector: &str) {
        let sql = "SELECT encrypted_jsonb->$1 FROM encrypted LIMIT 1";
        let actual = query_by::<Option<Value>>(sql, &selector).await;

        let expected = vec![None];
        assert_expected(&expected, &actual);

        let sql = format!("SELECT encrypted_jsonb  -> '{selector}' FROM encrypted LIMIT 1");
        let actual = simple_query_with_null(&sql).await;

        let expected = vec![None];
        assert_expected(&expected, &actual);
    }

    #[tokio::test]
    async fn jsonb_get_string_field_as_ciphertext() {
        trace();

        clear().await;
        insert_jsonb().await;

        let value = "hello".to_string();

        select_get_jsonb_field("string", value.to_owned()).await;

        // JSONPath selectors work with EQL fields
        select_get_jsonb_field("$.string", value.to_owned()).await;
    }

    #[tokio::test]
    async fn jsonb_get_numeric_field_as_ciphertext() {
        trace();

        clear().await;
        insert_jsonb().await;

        select_get_jsonb_field("number", 42).await;

        // JSONPath selectors work with EQL fields
        select_get_jsonb_field("$.number", 42).await;
    }

    #[tokio::test]
    async fn jsonb_get_numeric_array_field_as_ciphertext() {
        trace();

        clear().await;
        insert_jsonb().await;

        let value = serde_json::json!([42, 84]);

        select_get_jsonb_field("array_number", value.to_owned()).await;

        select_get_jsonb_field("$.array_number", value.to_owned()).await;
    }

    #[tokio::test]
    async fn jsonb_get_stringeric_array_field_as_ciphertext() {
        trace();

        clear().await;
        insert_jsonb().await;

        let value = serde_json::json!(["hello".to_string(), "world".to_string()]);

        select_get_jsonb_field("array_string", value.to_owned()).await;

        select_get_jsonb_field("$.array_string", value.to_owned()).await;
    }

    #[tokio::test]
    async fn jsonb_get_object_field_as_ciphertext() {
        trace();

        clear().await;
        insert_jsonb().await;

        let value = serde_json::json!({
            "number": 1815,
            "string": "world",
        });

        select_get_jsonb_field("nested", value.to_owned()).await;

        select_get_jsonb_field("$.nested", value.to_owned()).await;
    }

    #[tokio::test]
    async fn jsonb_get_unknown_field_as_ciphertext() {
        trace();

        clear().await;
        insert_jsonb().await;

        select_get_jsonb_field_null("blahvtha").await;
        select_get_jsonb_field_null("$.blahvtha").await;
    }
}
