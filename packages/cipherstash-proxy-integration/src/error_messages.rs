#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, trace, PROXY};

    #[tokio::test]
    async fn encrypted_column_with_no_configuration() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let encrypted_text = "hello@cipherstash.com";

        // let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        // client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "INSERT INTO encrypted (encrypted_unconfigured) VALUES ($1)";
        let result = client.query(sql, &[&encrypted_text]).await;

        assert!(result.is_err());

        if let Err(err) = result {
            let msg = err.to_string();
            assert_eq!(msg, "db error: ERROR: Column 'encrypted_unconfigured' in table 'encrypted' has no Encrypt configuration. For help visit https://github.com/cipherstash/proxy/docs/errors.md#encrypt-unknown-column");
        }
    }
}
