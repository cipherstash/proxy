#[cfg(test)]
mod tests {
    use crate::common::{clear, insert_jsonb, simple_query, trace};

    #[tokio::test]
    async fn array_containment() {
        trace();

        clear().await;

        insert_jsonb().await;

        let sql =
            "SELECT encrypted_jsonb @> '{ \"array_number\": [42, 84]}' FROM encrypted LIMIT 1";
        let contains: bool = simple_query(sql).await[0];

        assert!(contains);

        let sql =
            "SELECT encrypted_jsonb @> '{ \"array_number\": [41, 85]}' FROM encrypted LIMIT 1";
        let contains: bool = simple_query(sql).await[0];

        assert!(!contains);
    }
}
