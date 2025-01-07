#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, database_config_with_port, PROXY};
    use cipherstash_proxy::log;

    fn id() -> i64 {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        rng.gen_range(1..=i64::MAX)
    }

    #[tokio::test]
    async fn map_text() {
        log::init();

        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

        // generate random nunber between `1 and MAX`

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
        log::init();

        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

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
        log::init();

        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

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
        log::init();

        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

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
        log::init();

        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

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
}
