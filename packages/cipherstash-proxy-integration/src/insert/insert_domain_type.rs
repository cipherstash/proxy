#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, insert, query_by, random_id, trace, PROXY};
    use tokio_postgres::types::{FromSql, ToSql};

    #[derive(Debug, ToSql, FromSql, PartialEq)]
    #[postgres(name = "domain_type_with_check")]
    pub struct Domain(String);

    ///
    /// Tests insertion of custom domain type
    ///
    #[tokio::test]
    async fn insert_domain_type() {
        trace();

        let id = random_id();
        let encrypted_domain = Domain("ZZ".to_string());

        let sql = "INSERT INTO encrypted (id, plaintext_domain) VALUES ($1, $2)";
        insert(sql, &[&id, &encrypted_domain]).await;

        let sql = "SELECT plaintext_domain FROM encrypted WHERE id = $1";
        let result = query_by::<Domain>(sql, &id).await;

        let expected = vec![encrypted_domain];
        assert_eq!(expected, result);
    }

    ///
    /// Tests insertion of custom domain type with returned values
    ///
    #[tokio::test]
    async fn insert_domain_type_with_encrypted_and_returning() {
        trace();

        let id = random_id();
        let encrypted_domain = Domain("ZZ".to_string());
        let encrypted_text = "blah-vtha".to_string();

        let sql = "INSERT INTO encrypted (id, plaintext_domain, encrypted_text) VALUES ($1, $2, $3) RETURNING id, plaintext_domain, encrypted_text";

        let client = connect_with_tls(PROXY).await;
        let result = client
            .query(sql, &[&id, &encrypted_domain, &encrypted_text])
            .await
            .unwrap();

        assert_eq!(result.len(), 1);

        for row in result {
            let result_id: i64 = row.get("id");
            assert_eq!(id, result_id);

            let result_encrypted_text: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, result_encrypted_text);

            let result_domain: Domain = row.get("plaintext_domain");
            assert_eq!(encrypted_domain, result_domain);
        }
    }
}
