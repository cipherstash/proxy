#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, id, trace, PROXY};
    use chrono::NaiveDate;

    #[tokio::test]
    async fn map_text() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert!(rows.len() == 1);

        for row in rows {
            let result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }
    }

    #[tokio::test]
    async fn map_bool() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_bool: bool = true;

        let sql = "INSERT INTO encrypted (id, encrypted_bool) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_bool]).await.unwrap();

        let sql = "SELECT id, encrypted_bool FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert!(rows.len() == 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result_bool: bool = row.get("encrypted_bool");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_bool, result_bool);
        }
    }

    #[tokio::test]
    async fn map_int2() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int2: i16 = 42;

        let sql = "INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_int2]).await.unwrap();

        let sql = "SELECT id, encrypted_int2 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert!(rows.len() == 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result_int: i16 = row.get("encrypted_int2");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_int2, result_int);
        }
    }

    #[tokio::test]
    async fn map_int4() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int4: i32 = 42;

        let sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_int4]).await.unwrap();

        let sql = "SELECT id, encrypted_int4 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert!(rows.len() == 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result_int: i32 = row.get("encrypted_int4");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_int4, result_int);
        }
    }

    #[tokio::test]
    async fn map_int8() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int8: i64 = 42;

        let sql = "INSERT INTO encrypted (id, encrypted_int8) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_int8]).await.unwrap();

        let sql = "SELECT id, encrypted_int8 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert!(rows.len() == 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result_int: i64 = row.get("encrypted_int8");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_int8, result_int);
        }
    }

    #[tokio::test]
    async fn map_float8() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_float8: f64 = 42.00;

        let sql = "INSERT INTO encrypted (id, encrypted_float8) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_float8]).await.unwrap();

        let sql = "SELECT id, encrypted_float8 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert!(rows.len() == 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result: f64 = row.get("encrypted_float8");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_float8, result);
        }
    }

    #[tokio::test]
    async fn map_date() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_date = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();

        let sql = "INSERT INTO encrypted (id, encrypted_date) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_date]).await.unwrap();

        let sql = "SELECT id, encrypted_date FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert!(rows.len() == 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result: NaiveDate = row.get("encrypted_date");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_date, result);
        }
    }

    #[tokio::test]
    async fn map_jsonb() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_jsonb = serde_json::json!({"key": "value"});

        let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_jsonb]).await.unwrap();

        let sql = "SELECT id, encrypted_jsonb FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result: serde_json::Value = row.get("encrypted_jsonb");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_jsonb, result);
        }
    }

    #[tokio::test]
    async fn map_plaintext() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let plaintext = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id, &plaintext]).await.unwrap();

        let sql = "SELECT id, plaintext FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert!(rows.len() == 1);

        for row in rows {
            let result: String = row.get("plaintext");
            assert_eq!(plaintext, result);
        }
    }

    #[tokio::test]
    async fn map_all_with_wildcard() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
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

        assert!(rows.len() == 1);

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
