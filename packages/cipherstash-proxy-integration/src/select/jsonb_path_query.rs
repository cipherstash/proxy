#[cfg(test)]
mod tests {
    use crate::common::{clear, insert, query_by, random_id, trace};
    use crate::support::json_path::JsonPath;
    use bytes::BytesMut;
    use serde_json::{Number, Value};
    use tracing::info;

    async fn insert_jsonb() -> i64 {
        let encrypted_jsonb = serde_json::json!({
            "string": "hello",
            "number": 42,
            "nested": {
                "number": 1815,
                "string": "world",
            }
        });

        let id = random_id();
        let sql = format!("INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)");
        insert(&sql, &[&id, &encrypted_jsonb]).await;
        id
    }

    #[tokio::test]
    async fn select_jsonb_path_query_number() {
        trace();

        clear().await;

        let id = insert_jsonb().await;

        let selector = JsonPath::new("$.number");
        let sql = format!("SELECT jsonb_path_query(encrypted_jsonb, $1) FROM encrypted");

        let rows = query_by::<Value>(&sql, &selector).await;

        assert_eq!(rows.len(), 1);

        for row in rows {
            let expected = Value::from(42);
            assert_eq!(expected, row);
        }
    }

    #[tokio::test]
    async fn select_jsonb_path_query_string() {
        trace();

        clear().await;

        let id = insert_jsonb().await;

        let selector = JsonPath::new("$.nested.string");
        let sql = format!("SELECT jsonb_path_query(encrypted_jsonb, $1) FROM encrypted");

        let rows = query_by::<Value>(&sql, &selector).await;

        assert_eq!(rows.len(), 1);

        for row in rows {
            let expected = Value::from("world");
            assert_eq!(expected, row);
        }
    }

    #[tokio::test]
    async fn select_jsonb_path_query_value() {
        trace();

        clear().await;

        let id = insert_jsonb().await;

        let selector = JsonPath::new("$.nested");
        let sql = format!("SELECT jsonb_path_query(encrypted_jsonb, $1) FROM encrypted");

        let rows = query_by::<Value>(&sql, &selector).await;

        assert_eq!(rows.len(), 1);

        for row in rows {
            let expected = serde_json::json!({
                "number": 1815,
                "string": "world",
            });
            assert_eq!(expected, row);
        }
    }
}
