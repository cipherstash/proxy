#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};

    #[tokio::test]
    async fn decrypt_insert_returning_with_different_column_order() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let plaintext = "plaintext";
        let plaintext_date: Option<NaiveDate> = None;
        let encrypted_text = "hello@cipherstash.com";

        let sql =
            "INSERT INTO encrypted (id, plaintext, plaintext_date, encrypted_text) VALUES ($1, $2, $3, $4) RETURNING plaintext_date, id, plaintext, encrypted_text";
        let rows = client
            .query(sql, &[&id, &plaintext, &plaintext_date, &encrypted_text])
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let id_result: i64 = row.get("id");
            assert_eq!(id, id_result);

            let id_result: i64 = row.get(1);
            assert_eq!(id, id_result);

            let plaintext_result: String = row.get("plaintext");
            assert_eq!(plaintext, plaintext_result);

            let plaintext_result: String = row.get(2);
            assert_eq!(plaintext, plaintext_result);

            let plaintext_date_result: Option<NaiveDate> = row.get("plaintext_date");
            assert_eq!(None, plaintext_date_result);

            let plaintext_date_result: Option<NaiveDate> = row.get(0);
            assert_eq!(None, plaintext_date_result);

            let encrypted_text_result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, encrypted_text_result);

            let encrypted_text_result: String = row.get(3);
            assert_eq!(encrypted_text, encrypted_text_result);
        }
    }

    #[tokio::test]
    async fn decrypt_insert_returning_wildcard() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let plaintext = "plaintext";
        let encrypted_text = "hello@cipherstash.com";

        let sql =
            "INSERT INTO encrypted (id, plaintext, encrypted_text) VALUES ($1, $2, $3) RETURNING *";
        let rows = client
            .query(sql, &[&id, &plaintext, &encrypted_text])
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let id_result: i64 = row.get("id");
            assert_eq!(id, id_result);

            let id_result: i64 = row.get(0);
            assert_eq!(id, id_result);

            let plaintext_result: String = row.get("plaintext");
            assert_eq!(plaintext, plaintext_result);

            let plaintext_result: String = row.get(1);
            assert_eq!(plaintext, plaintext_result);

            let encrypted_text_result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, encrypted_text_result);

            let encrypted_text_result: String = row.get(4);
            assert_eq!(encrypted_text, encrypted_text_result);
        }
    }

    #[tokio::test]
    async fn decrypt_insert_returning() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let plaintext = "plaintext";
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, plaintext, encrypted_text) VALUES ($1, $2, $3) RETURNING encrypted_text";
        let rows = client
            .query(sql, &[&id, &plaintext, &encrypted_text])
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let encrypted_text_result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, encrypted_text_result);

            let encrypted_text_result: String = row.get(0);
            assert_eq!(encrypted_text, encrypted_text_result);
        }
    }
}
