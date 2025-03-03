#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use tokio_postgres::SimpleQueryMessage::{CommandComplete, Row};
    use tracing::info;

    #[tokio::test]
    async fn map_insert_null_param() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text: Option<String> = None;

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("encrypted_text");
            info!("result: {:?}", result);
            // assert_eq!(encrypted_text, result);
        }
    }

    #[tokio::test]
    async fn map_insert_null_literal() {
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, NULL)");

        client.simple_query(&sql).await.unwrap();

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.unwrap();

        if let Row(r) = &rows[1] {
            assert_eq!(Some("plain"), r.get(1));
        } else {
            panic!("Unexpected query results: {:?}", rows);
        }
    }
}
