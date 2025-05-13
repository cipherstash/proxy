#[cfg(test)]
mod tests {
    use tracing::{debug, info};

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

            // This is similar to below. The error message comes from tokio-postgres when Proxy
            // returns eql_v1_encrypted and the client cannot convert to a string.
            // If mapping errors are enabled (enable_mapping_errors or CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS),
            // then Proxy will return an error that says "Column X in table Y has no Encrypt configuration"
            assert_eq!(msg, "error serializing parameter 1: cannot convert between the Rust type `&str` and the Postgres type `jsonb`");
        } else {
            unreachable!();
        }
    }

    /// The error here is in the Tokio/Postgres layer
    /// The statement is valid and parses correctly, and the encrypted_date columns is Described as a date
    /// An i32 cannot be converted to a date and tokio_postgres returns an error
    /// See python tests for example with no client type checking
    #[tokio::test]
    async fn mapper_unsupported_parameter_type_with_date() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        // let encrypted_date = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();
        let encrypted_date: i32 = 2025;

        let sql = "INSERT INTO encrypted (id, encrypted_date) VALUES ($1, $2)";
        let result = client.query(sql, &[&id, &encrypted_date]).await;

        assert!(result.is_err());

        if let Err(err) = result {
            let msg = err.to_string();
            assert_eq!(msg, "error serializing parameter 1: cannot convert between the Rust type `i32` and the Postgres type `date`");
        } else {
            unreachable!();
        }
    }

    #[tokio::test]
    async fn invalid_sql_statement() {
        trace();

        reset_schema().await;

        let client = connect_with_tls(PROXY).await;

        let _reset = Reset;

        // Create a record
        // If select returns no results, no configuration is required
        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted id, encrypted_text VALUES ($1, $2)";
        let result = client.query(sql, &[&id, &encrypted_text]).await;

        assert!(result.is_err());

        if let Err(err) = result {
            let msg = err.to_string();
            info!("{}", msg);
            assert_eq!(msg, "db error: ERROR: sql parser error: Expected: SELECT, VALUES, or a subquery in the query body, found: id at Line: 1, Column: 23. For help visit https://github.com/cipherstash/proxy/blob/main/docs/errors.md#mapping-invalid-sql-statement");
        } else {
            unreachable!();
        }
    }
}
