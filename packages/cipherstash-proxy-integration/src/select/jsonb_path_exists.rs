#[cfg(test)]
mod tests {
    use crate::common::{clear, insert, insert_jsonb, query_by, random_id, simple_query, trace};
    use crate::support::assert::{assert_expected, assert_expected_as_string};
    use crate::support::json_path::JsonPath;
    use bytes::BytesMut;
    use serde::de::DeserializeOwned;
    use serde_json::{Number, Value};
    use tracing::info;

    #[tokio::test]
    async fn select_where_jsonb_path_exists() {
        trace();

        clear().await;

        let expected = insert_jsonb().await;

        let selector = JsonPath::new("$.number");

        let sql = format!(
            "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_exists(encrypted_jsonb, $1)"
        );

        let rows = query_by::<Value>(&sql, &selector).await;

        assert_eq!(rows.len(), 1);

        for row in rows {
            assert_eq!(expected, row);
        }
    }
}
