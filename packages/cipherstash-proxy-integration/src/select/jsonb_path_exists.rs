#[cfg(test)]
mod tests {
    use crate::common::{clear, insert_jsonb, query_by, simple_query, trace};
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    async fn select_jsonb(selector: &str, value: bool) {
        let selector = JsonPath::new(selector);
        let expected = vec![value];

        let sql = "SELECT jsonb_path_exists(encrypted_jsonb, $1) FROM encrypted";
        let actual = query_by::<bool>(sql, &selector).await;

        assert_expected(&expected, &actual);

        let sql = format!("SELECT jsonb_path_exists(encrypted_jsonb, '{selector}') FROM encrypted");
        let actual = simple_query::<bool>(&sql).await;

        assert_expected(&expected, &actual);
    }

    #[tokio::test]
    async fn select_jsonb_path_exists_number() {
        trace();

        clear().await;

        insert_jsonb().await;

        select_jsonb("$.number", true).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_exists_string() {
        trace();

        clear().await;

        insert_jsonb().await;

        select_jsonb("$.nested.string", true).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_exists_value() {
        trace();

        clear().await;

        insert_jsonb().await;

        let v = serde_json::json!({
            "number": 1815,
            "string": "world",
        });

        select_jsonb("$.nested", true).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_exists_with_unknown_selector() {
        trace();

        clear().await;

        insert_jsonb().await;

        select_jsonb("$.vtha", false).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_exists_with_alias() {
        trace();

        clear().await;

        insert_jsonb().await;

        let selector = JsonPath::new("$.nested");
        let expected = vec![true];

        let sql = "SELECT jsonb_path_exists(encrypted_jsonb, $1) as selected FROM encrypted";
        let actual = query_by::<bool>(sql, &selector).await;

        assert_expected(&expected, &actual);

        let sql = format!(
            "SELECT jsonb_path_exists(encrypted_jsonb, '{selector}') as selected FROM encrypted"
        );
        let actual = simple_query::<bool>(&sql).await;

        assert_expected(&expected, &actual);
    }
}
