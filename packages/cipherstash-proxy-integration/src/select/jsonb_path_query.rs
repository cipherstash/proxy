#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, insert, query_by, random_id, trace,
    };
    use serde_json::Value;
    use tracing::info;

    //   'SELECT eql_v2.jsonb_array_elements(eql_v2.jsonb_path_query(e, ''f510853730e1c3dbd31b86963f029dd5'')) as e FROM encrypted;');

    #[tokio::test]
    async fn select_jsonb_path_query() {
        trace();

        clear().await;

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

        let sql = format!("SELECT encrypted_jsonb::jsonb FROM encrypted WHERE id = $1");

        // let actual = query_by::<Value>(&sql, &id).await;

        // let expected = vec![encrypted_jsonb];
        // assert_eq!(actual, expected);

        let db_port = std::env::var("CS_DATABASE__PORT").unwrap().parse().unwrap();

        let client = connect_with_tls(db_port).await;
        let rows = client.query(&sql, &[&id]).await.unwrap();
        let results = rows.iter().map(|row| row.get(0)).collect::<Vec<Value>>();
        let result = results.first().unwrap();

        info!("Results: {:?}", result.get("sv".to_string()));
        // info!("Results: {:?}", results);

        let selector = "$.nested.string";
        let sql =
            format!("SELECT eql_v2.jsonb_path_query(encrypted_jsonb, $1)::jsonb FROM encrypted");

        let actual = query_by::<Value>(&sql, &selector).await;

        info!("Actual: {:?}", actual);

        // let expected = vec![encrypted_jsonb];
        // assert_eq!(actual, expected);
    }
}
