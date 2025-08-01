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

        let rows = client
            .query(
                "SELECT attname, atttypid FROM pg_catalog.pg_attribute
                WHERE attrelid IS NOT NULL
                AND NOT attisdropped AND attnum > 0
                ORDER BY attnum",
                &[],
            )
            .await
            .unwrap();

        assert!(
            !rows.is_empty(),
            "Expected non-empty result from pg_catalog.pg_attribute",
        );

        let rows = client
            .query("SELECT * FROM pg_catalog.pg_type", &[])
            .await
            .unwrap();

        assert!(
            !rows.is_empty(),
            "Expected non-empty result from pg_catalog.pg_type",
        );
    }
}
