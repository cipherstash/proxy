#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::{
        common::{clear, insert_jsonb, query_by, simple_query, trace},
        support::assert::assert_expected,
    };

    async fn select_contained_by_jsonb(value: Value, expected: bool) {
        let expected = vec![expected];

        let sql = "SELECT $1 <@ encrypted_jsonb FROM encrypted LIMIT 1";
        let contains = query_by::<bool>(sql, &value).await;

        assert_expected(&expected, &contains);

        let sql = format!("SELECT '{value}' <@ encrypted_jsonb FROM encrypted LIMIT 1");
        let contains = simple_query::<bool>(&sql).await;

        assert_expected(&expected, &contains);
    }

    #[tokio::test]
    async fn jsonb_contained_by_with_string() {
        trace();

        clear().await;
        insert_jsonb().await;

        let value = serde_json::json!({
            "string": "hello",
        });

        select_contained_by_jsonb(value, true).await;

        // Not contained
        let value = serde_json::json!({
            "string": "blah",
        });

        select_contained_by_jsonb(value, false).await;
    }

    #[tokio::test]
    async fn jsonb_contained_by_with_number() {
        trace();

        clear().await;
        insert_jsonb().await;

        let value = serde_json::json!({
            "number": 42,
        });

        select_contained_by_jsonb(value, true).await;

        // Not contained
        let value = serde_json::json!({
            "number": 11,
        });

        select_contained_by_jsonb(value, false).await;
    }

    #[tokio::test]
    async fn jsonb_contained_by_with_numeric_array() {
        trace();

        clear().await;
        insert_jsonb().await;

        let value = serde_json::json!({
            "array_number": [42, 84],
        });

        select_contained_by_jsonb(value, true).await;

        // Not contained
        let value = serde_json::json!({
            "array_number": [1, 2],
        });

        select_contained_by_jsonb(value, false).await;
    }

    #[tokio::test]
    async fn jsonb_contained_by_with_stringeric_array() {
        trace();

        clear().await;
        insert_jsonb().await;

        let value = serde_json::json!({
            "array_string": ["hello", "world"],
        });

        select_contained_by_jsonb(value, true).await;

        // Not contained
        let value = serde_json::json!({
            "array_string": ["blah", "vtha"],
        });

        select_contained_by_jsonb(value, false).await;
    }
}
