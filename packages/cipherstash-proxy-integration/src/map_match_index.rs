#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};

    #[tokio::test]
    async fn map_match_index_text() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_text])
            .await
            .expect("ok");

        let sql = "SELECT * FROM encrypted WHERE encrypted_text @> $1";
        let rows = client.query(sql, &[&"hello@"]).await.expect("ok");

        assert!(rows.len() == 1);

        for row in rows {
            let result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }
    }
}
