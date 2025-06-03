#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, PROXY};

    ///
    /// Tests access to pg_catalog
    /// Ensures metadata can be loaded from pg_catalog
    ///
    #[tokio::test]
    async fn select_from_pg_catalog() {
        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT attname, atttypid FROM pg_catalog.pg_attribute WHERE attrelid IS NOT NULL AND NOT attisdropped AND attnum > 0 ORDER BY attnum";
        let rows = client.query(sql, &[]).await.unwrap();

        assert!(
            !rows.is_empty(),
            "Expected non-empty result from pg_catalog.pg_attribute",
        );
    }
}
