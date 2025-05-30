#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, random_id, reset_schema, trace, PROXY};
    use chrono::NaiveDate;

    #[tokio::test]
    async fn map_all_with_wildcard() {
        trace();

        reset_schema().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let plaintext = "hello@cipherstash.com";
        let encrypted_text = "hello@cipherstash.com";
        let encrypted_bool = false;
        let encrypted_int2: i16 = 1;
        let encrypted_int4: i32 = 2;
        let encrypted_int8: i64 = 4;
        let encrypted_float8: f64 = 42.00;

        let sql = "INSERT INTO encrypted (id, plaintext, encrypted_text, encrypted_bool, encrypted_int2, encrypted_int4, encrypted_int8, encrypted_float8) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";
        client
            .query(
                sql,
                &[
                    &id,
                    &plaintext,
                    &encrypted_text,
                    &encrypted_bool,
                    &encrypted_int2,
                    &encrypted_int4,
                    &encrypted_int8,
                    &encrypted_float8,
                ],
            )
            .await
            .unwrap();

        let sql = "SELECT * FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("plaintext");
            assert_eq!(plaintext, result);

            let result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);

            let result: bool = row.get("encrypted_bool");
            assert_eq!(encrypted_bool, result);

            let result: i16 = row.get("encrypted_int2");
            assert_eq!(encrypted_int2, result);

            let result: i32 = row.get("encrypted_int4");
            assert_eq!(encrypted_int4, result);

            let result: i64 = row.get("encrypted_int8");
            assert_eq!(encrypted_int8, result);

            let result: f64 = row.get("encrypted_float8");
            assert_eq!(encrypted_float8, result);
        }
    }
}
