#[cfg(test)]
mod tests {
    use crate::common::{insert, query_by, random_id, trace};
    use tokio_postgres::types::{FromSql, ToSql};

    #[derive(Debug, ToSql, FromSql, PartialEq)]
    #[postgres(name = "domain_type_with_check")]
    pub struct Domain(String);

    ///
    /// Tests insertion of custom domain type
    ///
    #[tokio::test]
    async fn select_domain_type() {
        trace();

        let id = random_id();
        let encrypted_val = Domain("ZZ".to_string());

        let insert_sql = "INSERT INTO encrypted (id, plaintext_domain) VALUES ($1, $2)";
        insert(insert_sql, &[&id, &encrypted_val]).await;

        let select_sql = "SELECT plaintext_domain FROM encrypted WHERE id = $1";
        let result = query_by::<Domain>(select_sql, &id).await;

        let expected = vec![encrypted_val];
        assert_eq!(expected, result);
    }
}
