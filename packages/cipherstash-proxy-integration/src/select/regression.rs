#[cfg(test)]
mod tests {
    use crate::common::{insert, query_by, random_id, trace};

    ///
    /// IN clause subquery should be able to use ORDER BY
    ///
    #[tokio::test]
    async fn select_with_order_by_in_subquery() {
        trace();

        let id = random_id();
        let encrypted_text = "hello".to_string();

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        insert(sql, &[&id, &encrypted_text]).await;

        let sql = "SELECT encrypted_text FROM encrypted WHERE id IN (SELECT id FROM encrypted WHERE id = $1 ORDER BY id DESC LIMIT 10 OFFSET 0) GROUP BY encrypted_text";

        let result = query_by::<String>(sql, &id).await;

        let expected = vec![encrypted_text];
        assert_eq!(expected, result);
    }
}
