#[cfg(test)]
mod tests {
    use crate::common::{
        clear_with_client, connect_with_tls, insert_jsonb_with_client, query_by_with_client,
        simple_query_with_client, trace, PROXY,
    };
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use tokio_postgres::Client;

    async fn select_jsonb<T>(selector: &str, value: T, client: &Client)
    where
        T: DeserializeOwned,
        serde_json::Value: From<T>,
    {
        let selector = JsonPath::new(selector);
        let value = Value::from(value);

        let expected = vec![value];

        let sql = "SELECT jsonb_path_query(encrypted_jsonb, $1) FROM encrypted";
        let actual = query_by_with_client::<Value>(sql, &selector, client).await;

        assert_expected(&expected, &actual);

        let sql = format!("SELECT jsonb_path_query(encrypted_jsonb, '{selector}') FROM encrypted");
        let actual = simple_query_with_client::<Value>(&sql, client).await;

        assert_expected(&expected, &actual);
    }

    #[tokio::test]
    async fn select_jsonb_path_query_number() {
        trace();
        let client = connect_with_tls(PROXY).await;

        clear_with_client(&client).await;
        insert_jsonb_with_client(&client).await;

        select_jsonb("$.number", 42, &client).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_query_string() {
        trace();
        let client = connect_with_tls(PROXY).await;

        clear_with_client(&client).await;
        insert_jsonb_with_client(&client).await;

        select_jsonb("$.nested.string", "world".to_string(), &client).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_query_value() {
        trace();
        let client = connect_with_tls(PROXY).await;

        clear_with_client(&client).await;
        insert_jsonb_with_client(&client).await;

        let v = serde_json::json!({
            "number": 1815,
            "string": "world",
        });

        select_jsonb("$.nested", v, &client).await;
    }

    #[tokio::test]
    async fn select_jsonb_path_query_with_unknown() {
        trace();
        let client = connect_with_tls(PROXY).await;

        clear_with_client(&client).await;
        insert_jsonb_with_client(&client).await;

        let selector = JsonPath::new("$.vtha");

        let expected = vec![];

        let sql = "SELECT jsonb_path_query(encrypted_jsonb, $1) as selected FROM encrypted";
        let actual = query_by_with_client::<Value>(sql, &selector, &client).await;

        assert_expected(&expected, &actual);

        let sql = format!(
            "SELECT jsonb_path_query(encrypted_jsonb, '{selector}') as selected FROM encrypted"
        );
        let actual = simple_query_with_client::<Value>(&sql, &client).await;

        assert_expected(&expected, &actual);
    }

    #[tokio::test]
    async fn select_jsonb_path_query_with_alias() {
        trace();
        let client = connect_with_tls(PROXY).await;

        clear_with_client(&client).await;
        insert_jsonb_with_client(&client).await;

        let value = serde_json::json!({
            "number": 1815,
            "string": "world",
        });

        let selector = JsonPath::new("$.nested");

        let expected = vec![value];

        let sql = "SELECT jsonb_path_query(encrypted_jsonb, $1) as selected FROM encrypted";
        let actual = query_by_with_client::<Value>(sql, &selector, &client).await;

        assert_expected(&expected, &actual);

        let sql = format!(
            "SELECT jsonb_path_query(encrypted_jsonb, '{selector}') as selected FROM encrypted"
        );
        let actual = simple_query_with_client::<Value>(&sql, &client).await;

        assert_expected(&expected, &actual);
    }
}
