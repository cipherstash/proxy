#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, random_id, PROXY};

    #[tokio::test]
    async fn frontend_error_does_not_crash_connection() {
        let client = connect_with_tls(PROXY).await;

        // Statement has the wrong column name
        let sql = format!(
            "INSERT INTO encrypted (id, encrypted) VALUES ({}, 'foo@example.net')",
            random_id()
        );

        let result = client.simple_query(&sql).await;

        assert!(result.is_err());
        let error = result.unwrap_err();

        // The connection should not be closed
        assert!(!error.is_closed());

        // And we can still use the connection
        let sql = format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({}, 'foo@example.net')",
            random_id()
        );
        let result = client.simple_query(&sql).await;

        assert!(result.is_ok());
    }
}
