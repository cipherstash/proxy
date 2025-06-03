#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, PROXY};

    ///
    /// Tests unmappble statements return an error in tests.
    ///
    /// `enable_mapping_errors` should be `true` in the test configuration.`
    ///
    /// Test ensures that unmappable SQL statements return an error
    ///
    #[tokio::test]
    async fn unmappable_error() {
        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT blah FROM vtha";
        let result = client.query(sql, &[]).await;

        assert!(
            result.is_err(),
            "Expected unmappble SQL statement to return an error",
        );
    }
}
