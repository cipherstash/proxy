#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, PROXY};

    #[tokio::test]
    async fn empty_result_regression() {
        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT ''";

        let rows = client.query(sql, &[]).await.unwrap();

        assert_eq!(rows.len(), 1);

        let empty: String = rows[0].get(0);
        assert_eq!(empty, "");
    }
}
