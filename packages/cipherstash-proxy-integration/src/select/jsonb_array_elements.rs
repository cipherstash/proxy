#[cfg(test)]
mod tests {
    use crate::common::{
        clear_with_client, connect_with_tls, insert_jsonb_with_client, query_by_with_client,
        simple_query_with_client, trace, PROXY,
    };
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde_json::Value;
    use tokio_postgres::Client;

    async fn select_jsonb(selector: &str, expected: &[Value], client: &Client) {
        let selector = JsonPath::new(selector);

        let sql =
            "SELECT jsonb_array_elements(jsonb_path_query(encrypted_jsonb, $1)) FROM encrypted";
        let actual = query_by_with_client::<Value>(sql, &selector, client).await;

        assert_expected(expected, &actual);

        let sql = format!("SELECT jsonb_array_elements(jsonb_path_query(encrypted_jsonb, '{selector}')) FROM encrypted");
        let actual = simple_query_with_client::<Value>(&sql, client).await;

        assert_expected(expected, &actual);
    }

    #[tokio::test]
    async fn select_jsonb_array_elements_with_string() {
        trace();
        let client = connect_with_tls(PROXY).await;

        clear_with_client(&client).await;
        insert_jsonb_with_client(&client).await;

        let expected = vec![Value::from("hello"), Value::from("world")];
        select_jsonb("$.array_string[@]", &expected, &client).await;
    }

    #[tokio::test]
    async fn select_jsonb_array_elements_with_numeric() {
        trace();
        let client = connect_with_tls(PROXY).await;

        clear_with_client(&client).await;
        insert_jsonb_with_client(&client).await;

        let expected = vec![Value::from(42), Value::from(84)];
        select_jsonb("$.array_number[@]", &expected, &client).await;
    }

    #[tokio::test]
    async fn select_jsonb_array_elements_with_unknown_field() {
        trace();
        let client = connect_with_tls(PROXY).await;

        clear_with_client(&client).await;
        insert_jsonb_with_client(&client).await;

        let expected = vec![];
        select_jsonb("$.blah", &expected, &client).await;
    }
}
