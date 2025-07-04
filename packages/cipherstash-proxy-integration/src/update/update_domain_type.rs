#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, execute_query, query_by, random_id, trace, PROXY};
    use tokio_postgres::types::{FromSql, ToSql};

    #[derive(Debug, ToSql, FromSql, PartialEq)]
    #[postgres(name = "domain_type_with_check")]
    pub struct Domain(String);

    ///
    /// Tests update of custom domain type
    ///
    #[tokio::test]
    async fn update_domain_type() {
        trace();

        let id = random_id();
        let initial_domain = Domain("AA".to_string());
        let updated_domain = Domain("ZZ".to_string());

        // First insert a record
        let sql = "INSERT INTO encrypted (id, plaintext_domain) VALUES ($1, $2)";
        execute_query(sql, &[&id, &initial_domain]).await;

        // Then update it
        let sql = "UPDATE encrypted SET plaintext_domain = $1 WHERE id = $2";
        execute_query(sql, &[&updated_domain, &id]).await;

        let sql = "SELECT plaintext_domain FROM encrypted WHERE id = $1";
        let result = query_by::<Domain>(sql, &id).await;

        let expected = vec![updated_domain];
        assert_eq!(expected, result);
    }

    ///
    /// Tests update of custom domain type with returned values
    ///
    #[tokio::test]
    async fn update_domain_type_with_encrypted_and_returning() {
        trace();

        let id = random_id();
        let initial_domain = Domain("AA".to_string());
        let initial_text = "initial-text".to_string();
        let updated_domain = Domain("ZZ".to_string());
        let updated_text = "updated-text".to_string();

        // First insert a record
        let sql =
            "INSERT INTO encrypted (id, plaintext_domain, encrypted_text) VALUES ($1, $2, $3)";
        execute_query(sql, &[&id, &initial_domain, &initial_text]).await;

        // Then update with RETURNING clause
        let sql = "UPDATE encrypted SET plaintext_domain = $1, encrypted_text = $2 WHERE id = $3 RETURNING id, plaintext_domain, encrypted_text";

        let client = connect_with_tls(PROXY).await;
        let result = client
            .query(sql, &[&updated_domain, &updated_text, &id])
            .await
            .unwrap();

        assert_eq!(result.len(), 1);

        for row in result {
            let result_id: i64 = row.get("id");
            assert_eq!(id, result_id);

            let result_encrypted_text: String = row.get("encrypted_text");
            assert_eq!(updated_text, result_encrypted_text);

            let result_domain: Domain = row.get("plaintext_domain");
            assert_eq!(updated_domain, result_domain);
        }
    }
}
