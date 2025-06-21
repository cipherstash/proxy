#[cfg(test)]
mod tests {
    use std::panic;

    use crate::common::{
        clear, connect_with_tls, execute_query, query, query_by, random_id, trace, PROXY,
    };
    use serde_json::Value;
    use tokio_postgres::types::{FromSql, ToSql};
    use tracing::info;

    #[derive(Clone, Debug, ToSql, FromSql, PartialEq)]
    #[postgres(name = "eql_v2_encrypted")]
    pub struct EqlEncrypted {
        pub data: Value,
    }

    ///
    /// Tests disabling mapping
    ///
    #[tokio::test]
    async fn insert_with_set_unsafe_disable_mapping() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_text = "hello".to_string();

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
        client.query(sql, &[]).await.unwrap();

        let insert_sql = format!("INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)");
        let result = client.query(&insert_sql, &[&id, &encrypted_text]).await;

        // This error is actually a `WrongType` error from the tokio client as encrypted_text is actually eql_v2_encrypted
        assert!(result.is_err());

        // Force the eql_v2_encrypted type
        let encrypted = EqlEncrypted {
            data: Value::from(encrypted_text.to_owned()),
        };

        let result = client.query(&insert_sql, &[&id, &encrypted]).await;

        // TODO: this should be an error, check the constraint
        assert!(result.is_ok());

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false";
        client.query(sql, &[]).await.unwrap();

        let id = random_id();
        let result = client.query(&insert_sql, &[&id, &encrypted_text]).await;
        assert!(result.is_ok());
    }

    ///
    /// Tests disabling mapping
    ///
    #[tokio::test]
    async fn select_with_set_unsafe_disable_mapping() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();

        let encrypted_text = "hello".to_string();

        let sql = format!("INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)");
        execute_query(&sql, &[&id, &encrypted_text]).await;

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
        client.query(sql, &[]).await.unwrap();

        // Data should not be decrypted
        let sql = "SELECT encrypted_text FROM encrypted";
        let rows = client.query(sql, &[]).await.unwrap();

        // Check that rows are raw EqlEncrypted
        // If the statement was mapped, encrypted_text would be a String and this is a panic
        let actual = rows
            .iter()
            .map(|row| row.get(0))
            .collect::<Vec<EqlEncrypted>>();

        assert_eq!(actual.len(), 1);

        // Turn mapping back on and regular services resume
        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false";
        client.query(sql, &[]).await.unwrap();

        let sql = "SELECT encrypted_text FROM encrypted";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();

        assert_eq!(actual.len(), 1);
        let expected = vec![encrypted_text];
        assert_eq!(expected, actual);
    }
}
