#[cfg(test)]
mod tests {
    use crate::common::{
        connect_with_tls, insert, query_by, random_id, simple_query, trace, PROXY,
    };
    use tokio_postgres::types::{FromSql, ToSql};
    use tracing::info;

    #[derive(Debug, ToSql, FromSql, PartialEq)]
    #[postgres(name = "domain_type_with_check")]
    pub struct Domain(String);

    ///
    /// Tests insertion of custom domain type
    ///
    #[tokio::test]
    async fn select_with_index() {
        trace();

        // let id = random_id();
        // let encrypted_val = Domain("ZZ".to_string());

        // CREATE INDEX ON encrypted (e eql_v2.encrypted_operator_class);
        // SELECT ore.e FROM ore WHERE id = 42 INTO ore_term;

        for n in 1..=10 {
            let id = random_id();

            let encrypted_text = format!("hello_{}", n);

            let sql = format!("INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)");
            insert(&sql, &[&id, &encrypted_text]).await;
        }

        let client = connect_with_tls(PROXY).await;

        let sql = "CREATE INDEX ON encrypted (encrypted_text eql_v2.encrypted_operator_class)";
        let _ = client.simple_query(sql).await;

        // let sql =
        //     "EXPLAIN ANALYZE SELECT encrypted_text FROM encrypted WHERE encrypted_text <= '{\"hm\": \"abc\"}'::jsonb::eql_v2_encrypted";
        // let result = simple_query::<String>(sql).await;

        let sql = "EXPLAIN ANALYZE SELECT encrypted_text FROM encrypted WHERE encrypted_text <= $1";

        let encrypted_text = "hello_10".to_string();
        let result = query_by::<String>(sql, &encrypted_text).await;

        info!("Result: {:?}", result);

        // let expected = vec![encrypted_val];
        // assert_eq!(expected, result);
    }
}
