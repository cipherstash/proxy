#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, PROXY};

    #[tokio::test]
    async fn map_concat_regression() {
        let client = connect_with_tls(PROXY).await;

        let sql = "UPDATE encrypted SET encrypted_text = encrypted_text || 'suffix';";

        client
            .query(sql, &[])
            .await
            .expect_err("expected update to fail, but it succeeded");
    }
}
