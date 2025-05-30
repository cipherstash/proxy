#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, PROXY};

    #[tokio::test]
    async fn map_concat_regression() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        let id = random_id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "UPDATE encrypted SET encrypted_text = encrypted_text || 'suffix';";

        client
            .query(sql, &[])
            .await
            .expect_err("expected update to fail, but it succeeded");
    }
}
