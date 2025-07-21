#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, insert, query_by_params, random_id, simple_query, trace, PROXY,
    };
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use tracing::info;

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
    async fn select_jsonb_where_string_equality_with_field_name() {
        trace();

        clear().await;
        insert_jsonb().await;

        let client = connect_with_tls(PROXY).await;

        let selector = "string";
        let value = Value::from("A");

        let expected = vec![serde_json::json!({
            "string": "A",
            "number": 1,
        })];

        // WHERE encrypted_jsonb->'number' = 1
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 = $2";

        let rows = query_by_params::<Value>(sql, &[&selector, &value]).await;

        assert_eq!(rows.len(), 1);
        assert_eq!(expected, rows);

        let sql =
            format!(
                "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> '{selector}' = '{value}'"
        );
        let rows = simple_query::<Value>(&sql).await;

        assert_eq!(rows.len(), 1);
        assert_eq!(expected, rows);
    }

    #[tokio::test]
    async fn select_jsonb_where_numeric_equality_with_field_name() {
        trace();

        clear().await;
        insert_jsonb().await;

        let client = connect_with_tls(PROXY).await;

        let selector = "number";
        let value = Value::from(4);

        let expected = vec![serde_json::json!({
            "string": "D",
            "number": 4,
        })];

        // WHERE encrypted_jsonb->'number' = 1
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 = $2";

        let rows = query_by_params::<Value>(sql, &[&selector, &value]).await;

        assert_eq!(rows.len(), 1);
        assert_eq!(expected, rows);

        let sql =
            format!(
                "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> '{selector}' = '{value}'"
        );
        let rows = simple_query::<Value>(&sql).await;

        assert_eq!(rows.len(), 1);
        assert_eq!(expected, rows);
    }

    #[tokio::test]
    async fn select_jsonb_where_string_equality_with_jsonb_path_query_first() {
        trace();

        clear().await;
        insert_jsonb().await;

        let client = connect_with_tls(PROXY).await;

        let selector = JsonPath::new("$.string");

        let value = Value::from("B");

        let expected = vec![serde_json::json!({
            "string": "B",
            "number": 2,
        })];

        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, $1) = $2";

        let rows = query_by_params::<Value>(sql, &[&selector, &value]).await;

        assert_eq!(rows.len(), 1);
        assert_eq!(expected, rows);

        let sql = format!(
                "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, '{selector}') = '{value}'"
        );
        let rows = simple_query::<Value>(&sql).await;

        assert_eq!(rows.len(), 1);
        assert_eq!(expected, rows);
    }

    #[tokio::test]
    async fn select_jsonb_where_numeric_equality_with_jsonb_path_query_first() {
        trace();

        clear().await;
        insert_jsonb().await;

        let client = connect_with_tls(PROXY).await;

        let selector = JsonPath::new("$.number");

        let value = Value::from(3);

        let expected = vec![serde_json::json!({
            "string": "C",
            "number": 3,
        })];

        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, $1) = $2";

        let rows = query_by_params::<Value>(sql, &[&selector, &value]).await;

        assert_eq!(rows.len(), 1);
        assert_eq!(expected, rows);

        let sql = format!(
                "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, '{selector}') = '{value}'"
        );
        let rows = simple_query::<Value>(&sql).await;

        assert_eq!(rows.len(), 1);
        assert_eq!(expected, rows);
    }
}
