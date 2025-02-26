#[cfg(test)]
mod tests {
    use tracing::debug;

    use crate::common::{clear, connect_with_tls, id, reset_schema, trace, PROXY};

    struct Reset;

    impl Drop for Reset {
        fn drop(&mut self) {
            debug!("Reset schema");
            tokio::spawn(async {
                reset_schema().await;
                debug!("Reset schema complete");
            });
        }
    }

    #[tokio::test]
    async fn encrypted_column_not_defined_in_schema() {
        trace();

        clear().await;

        let _reset = Reset;

        let id = id();

        let client = connect_with_tls(PROXY).await;

        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_unconfigured) VALUES ($1, $2)";
        let result = client.query(sql, &[&id, &encrypted_text]).await;

        assert!(result.is_err());

        if let Err(err) = result {
            let msg = err.to_string();
            assert_eq!(msg, "db error: ERROR: column \"encrypted_unconfigured\" of relation \"encrypted\" does not exist");
        } else {
            unreachable!();
        }
    }

    #[tokio::test]
    async fn encrypted_column_with_no_configuration() {
        trace();

        reset_schema().await;

        let client = connect_with_tls(PROXY).await;

        let _reset = Reset;

        // Create a record
        // If select returns no results, no configuration is required
        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO unconfigured (id, encrypted_unconfigured) VALUES ($1, $2)";
        let result = client.query(sql, &[&id, &encrypted_text]).await;

        assert!(result.is_err());

        if let Err(err) = result {
            let msg = err.to_string();

            assert_eq!(msg, "db error: ERROR: Column 'encrypted_unconfigured' in table 'unconfigured' has no Encrypt configuration. For help visit https://github.com/cipherstash/proxy/docs/errors.md#encrypt-unknown-column");
        } else {
            unreachable!();
        }
    }
}
