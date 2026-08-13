#[cfg(test)]
mod tests {
    use crate::common::{assert_encrypted_text, clear, execute_query, query_by, random_id, trace};

    #[tokio::test]
    async fn conflict_update_encrypts_excluded_value() {
        trace();
        clear().await;

        let id = random_id();
        let initial = "initial value".to_string();
        let updated = "updated value".to_string();
        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2) \
                   ON CONFLICT (id) DO UPDATE SET encrypted_text = excluded.encrypted_text";

        execute_query(sql, &[&id, &initial]).await;
        execute_query(sql, &[&id, &updated]).await;

        assert_eq!(
            query_by::<String>("SELECT encrypted_text FROM encrypted WHERE id = $1", &id).await,
            vec![updated.clone()]
        );
        assert_encrypted_text(id, "encrypted_text", &updated).await;
    }
}
