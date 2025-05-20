#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tracing::info;

    use crate::common::{clear, connect_with_tls, id, reset_schema, trace, PROXY};

    async fn seed_jsonb_data() {
        let mut map: HashMap<i64, &str> = HashMap::new();

        for (num, word) in (10..=100).step_by(10).zip([
            "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India",
            "Juliet",
        ]) {
            map.insert(num, word);
        }

        let client = connect_with_tls(PROXY).await;
        for i in (10..=100).step_by(10) {
            // let key = format!()

            let id: i64 = i;

            let s = map.get(&i).unwrap();

            let encrypted_jsonb = serde_json::json!({
                "string": s,
                "number": i,
                "nested": {
                    "number": i,
                    "string": s,
                }
            });

            // info!("{encrypted_jsonb:?}");

            let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
            client.query(sql, &[&id, &encrypted_jsonb]).await.unwrap();
        }
    }

    #[tokio::test]
    async fn lt_with_jsonb_path_query() {
        trace();

        clear().await;

        seed_jsonb_data().await;

        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb->'number' < $1";
        // let sql = "SELECT encrypted_jsonb FROM encrypted WHERE eql_v1.jsonb_path_query_first(encrypted_jsonb, '$.number') < $1";

        // let rows = client.query(sql, &[&n]).await.unwrap();
        let rows = client.query(sql, &[]).await.unwrap();

        // assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("encrypted_jsonb");
            info!("{result}");
            // assert_eq!(encrypted_text, result);
        }
    }
}
