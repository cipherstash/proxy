#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, PROXY};
    use chrono::NaiveDate;
    use cipherstash_proxy::{config::LogConfig, log};

    fn id() -> i64 {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        rng.gen_range(1..=i64::MAX)
    }

    #[tokio::test]
    async fn map_text() {
        log::init(LogConfig::default());

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_text])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

        assert!(rows.len() == 1);

        for row in rows {
            let result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }
    }

    #[tokio::test]
    async fn map_bool() {
        log::init(LogConfig::default());

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_bool: bool = true;

        let sql = "INSERT INTO encrypted (id, encrypted_bool) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_bool])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_bool FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

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
        log::init(LogConfig::default());

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int2: i16 = 42;

        let sql = "INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_int2])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_int2 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

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
        log::init(LogConfig::default());

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int4: i32 = 42;

        let sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_int4])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_int4 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

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
        log::init(LogConfig::default());

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int8: i64 = 42;

        let sql = "INSERT INTO encrypted (id, encrypted_int8) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_int8])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_int8 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

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
        log::init(LogConfig::default());

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_float8: f64 = 42.00;

        let sql = "INSERT INTO encrypted (id, encrypted_float8) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_float8])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_float8 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

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
        log::init(LogConfig::default());

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_date = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();

        let sql = "INSERT INTO encrypted (id, encrypted_date) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_date])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_date FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

        assert!(rows.len() == 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            let result: NaiveDate = row.get("encrypted_date");

            assert_eq!(id, result_id);
            assert_eq!(encrypted_date, result);
        }
    }

    #[tokio::test]
    async fn map_plaintext() {
        log::init(LogConfig::default());

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let plaintext = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id, &plaintext]).await.expect("ok");

        let sql = "SELECT id, plaintext FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

        assert!(rows.len() == 1);

        for row in rows {
            let result: String = row.get("plaintext");
            assert_eq!(plaintext, result);
        }
    }

    #[tokio::test]
    async fn map_all_with_wildcard() {
        log::init(LogConfig::default());

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
            .expect("ok");

        let sql = "SELECT * FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

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
