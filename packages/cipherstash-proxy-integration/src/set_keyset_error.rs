/// IMPORTANT
/// IMPORTANT
///
/// This test assumes that `CS_DEFAULT_KEYSET_ID`` has been set
///
/// Do not move into the multitenant module,
///
/// The mise integration task splits the `multitenant` tests out so that the config can be changed
///
#[cfg(test)]
mod tests {
    use tracing::info;

    use crate::common::{connect_with_tls, trace, PROXY};

    /// Helper function to assert that a result contains the expected "Cannot SET CIPHERSTASH.KEYSET" error
    fn assert_keyset_error<T>(result: Result<T, tokio_postgres::Error>) {
        if let Err(err) = result {
            let msg = err.to_string();
            assert_eq!(msg, "db error: FATAL: Cannot SET CIPHERSTASH.KEYSET if a default keyset has been configured. For help visit https://github.com/cipherstash/proxy/blob/main/docs/errors.md#encrypt-unexpected-set-keyset");
        } else {
            unreachable!();
        }
    }

    /// Tests error handling of unknown keyset id
    #[tokio::test]
    async fn set_keyset_id_with_default_config_error() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let sql = "SET CIPHERSTASH.KEYSET_ID = '2cace9db-3a2a-4b46-a184-ba412b3e0730'";

        let result = client.query(sql, &[]).await;
        info!(?result);
        assert!(result.is_err());

        assert_keyset_error(result);

        let result = client.simple_query(sql).await;
        assert!(result.is_err());

        assert_keyset_error(result);
    }

    /// Tests error handling of unknown keyset id
    #[tokio::test]
    async fn set_keyset_name_with_default_config_error() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let sql = "SET CIPHERSTASH.KEYSET_NAME = 'tenant-1'";

        let result = client.query(sql, &[]).await;
        assert!(result.is_err());

        assert_keyset_error(result);

        let result = client.simple_query(sql).await;
        assert!(result.is_err());

        assert_keyset_error(result);
    }
}
