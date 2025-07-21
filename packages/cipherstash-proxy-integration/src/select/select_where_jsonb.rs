#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, insert, query_by, query_by_params, random_id, simple_query, trace,
        PROXY,
    };
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use tracing::info;

    async fn select_jsonb_where<T>(selector: &str, value: T)
    where
        T: DeserializeOwned + std::fmt::Debug,
        serde_json::Value: From<T>,
    {
        let selector = JsonPath::new(selector);

        let expected = vec![value];

        // info!(?actual);
        // assert_expected(&expected, &actual);

        // let sql = format!("SELECT jsonb_path_query(encrypted_jsonb, '{selector}') FROM encrypted");
        // let actual = simple_query::<Value>(&sql).await;

        // assert_expected(&expected, &actual);
    }

    // async fn insert_encrypted_value<T>(col: &str, val: &T)
    // where
    //     T: ToSql + Sync + Send + 'static,
    // {
    //     let id = random_id();
    //     let sql = format!("INSERT INTO encrypted (id, {col}) VALUES ($1, $2)");
    //     execute_query(&sql, &[&id, &val]).await;
    // }

    pub async fn insert_jsonb() {
        for n in 1..=5 {
            // let n = 1;
            let id = random_id();
            let s = ((b'A' + (n - 1) as u8) as char).to_string();

            let encrypted_jsonb = serde_json::json!({
                "string": s,
                "number": n,
            });

            info!(?encrypted_jsonb);

            let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
            insert(sql, &[&id, &encrypted_jsonb]).await;
        }
    }

    #[tokio::test]
    async fn select_jsonb_where_get_numeric_field_by_name() {
        trace();

        clear().await;

        insert_jsonb().await;

        let client = connect_with_tls(PROXY).await;

        let selector = "number";
        let value = Value::from(1);

        let expected = vec![serde_json::json!({
            "string": "A",
            "number": 1,
        })];

        // WHERE encrypted_jsonb->'number' = 1
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 = $2";

        let rows = query_by_params::<Value>(sql, &[&selector, &value]).await;
        assert_eq!(rows.len(), 1);

        assert_eq!(expected, rows);

        info!(?rows);
    }

    // #[tokio::test]
    // async fn select_jsonb_path_query_string() {
    //     trace();

    //     clear().await;

    //     insert_jsonb().await;

    //     select_jsonb("$.nested.string", "world".to_string()).await;
    // }

    // #[tokio::test]
    // async fn select_jsonb_path_query_value() {
    //     trace();

    //     clear().await;

    //     insert_jsonb().await;

    //     let v = serde_json::json!({
    //         "number": 1815,
    //         "string": "world",
    //     });

    //     select_jsonb("$.nested", v).await;
    // }

    // #[tokio::test]
    // async fn select_jsonb_path_query_with_unknown() {
    //     trace();

    //     clear().await;

    //     insert_jsonb().await;

    //     let selector = JsonPath::new("$.vtha");

    //     let expected = vec![];

    //     let sql = "SELECT jsonb_path_query(encrypted_jsonb, $1) as selected FROM encrypted";
    //     let actual = query_by::<Value>(sql, &selector).await;

    //     assert_expected(&expected, &actual);

    //     let sql = format!(
    //         "SELECT jsonb_path_query(encrypted_jsonb, '{selector}') as selected FROM encrypted"
    //     );
    //     let actual = simple_query::<Value>(&sql).await;

    //     assert_expected(&expected, &actual);
    // }

    // #[tokio::test]
    // async fn select_jsonb_path_query_with_alias() {
    //     trace();

    //     clear().await;

    //     insert_jsonb().await;

    //     let value = serde_json::json!({
    //         "number": 1815,
    //         "string": "world",
    //     });

    //     let selector = JsonPath::new("$.nested");

    //     let expected = vec![value];

    //     let sql = "SELECT jsonb_path_query(encrypted_jsonb, $1) as selected FROM encrypted";
    //     let actual = query_by::<Value>(sql, &selector).await;

    //     assert_expected(&expected, &actual);

    //     let sql = format!(
    //         "SELECT jsonb_path_query(encrypted_jsonb, '{selector}') as selected FROM encrypted"
    //     );
    //     let actual = simple_query::<Value>(&sql).await;

    //     assert_expected(&expected, &actual);
    // }
}
