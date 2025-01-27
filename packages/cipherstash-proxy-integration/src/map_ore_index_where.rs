#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use chrono::NaiveDate;

    #[tokio::test]
    async fn map_ore_where_int2() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int2: i16 = 42;

        let low: i16 = 40;
        let high: i16 = 99;

        let sql = "INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_int2])
            .await
            .expect("ok");

        let sql = "SELECT encrypted_int2 FROM encrypted WHERE encrypted_int2 > $1";
        let rows = client.query(sql, &[&low]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i16 = row.get("encrypted_int2");
            assert_eq!(encrypted_int2, result_int);
        }

        let sql = "SELECT encrypted_int2 FROM encrypted WHERE encrypted_int2 < $1";
        let rows = client.query(sql, &[&high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i16 = row.get("encrypted_int2");
            assert_eq!(encrypted_int2, result_int);
        }

        let sql = "SELECT encrypted_int2 FROM encrypted WHERE encrypted_int2 > $1 AND encrypted_int2 < $2";
        let rows = client.query(sql, &[&low, &high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i16 = row.get("encrypted_int2");
            assert_eq!(encrypted_int2, result_int);
        }
    }

    #[tokio::test]
    async fn map_ore_where_int4() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int4: i32 = 42;

        let low: i32 = 1;
        let high: i32 = 99;

        let sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_int4])
            .await
            .expect("ok");

        let sql = "SELECT encrypted_int4 FROM encrypted WHERE encrypted_int4 > $1";
        let rows = client.query(sql, &[&low]).await.expect("ok");
        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i32 = row.get("encrypted_int4");
            assert_eq!(encrypted_int4, result_int);
        }

        let sql = "SELECT encrypted_int4 FROM encrypted WHERE encrypted_int4 < $1";
        let rows = client.query(sql, &[&high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i32 = row.get("encrypted_int4");
            assert_eq!(encrypted_int4, result_int);
        }

        let sql = "SELECT encrypted_int4 FROM encrypted WHERE encrypted_int4 > $1 AND encrypted_int4 < $2";
        let rows = client.query(sql, &[&low, &high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i32 = row.get("encrypted_int4");
            assert_eq!(encrypted_int4, result_int);
        }
    }

    #[tokio::test]
    async fn map_ore_where_int8() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_int8: i64 = 42;

        let low: i64 = 1;
        let high: i64 = 99;

        let sql = "INSERT INTO encrypted (id, encrypted_int8) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_int8])
            .await
            .expect("ok");

        let sql = "SELECT encrypted_int8 FROM encrypted WHERE encrypted_int8 > $1";
        let rows = client.query(sql, &[&low]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i64 = row.get("encrypted_int8");
            assert_eq!(encrypted_int8, result_int);
        }

        let sql = "SELECT encrypted_int8 FROM encrypted WHERE encrypted_int8 < $1";
        let rows = client.query(sql, &[&high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i64 = row.get("encrypted_int8");
            assert_eq!(encrypted_int8, result_int);
        }

        let sql = "SELECT encrypted_int8 FROM encrypted WHERE encrypted_int8 > $1 AND encrypted_int8 < $2 ";
        let rows = client.query(sql, &[&low, &high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result_int: i64 = row.get("encrypted_int8");
            assert_eq!(encrypted_int8, result_int);
        }
    }

    #[tokio::test]
    async fn map_ore_where_float8() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_float8: f64 = 42.00;

        let low: f64 = 1.0;
        let high: f64 = 99.00;

        let sql = "INSERT INTO encrypted (id, encrypted_float8) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_float8])
            .await
            .expect("ok");

        let sql = "SELECT * FROM encrypted WHERE encrypted_float8 > $1";
        let rows = client.query(sql, &[&low]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result: f64 = row.get("encrypted_float8");
            assert_eq!(encrypted_float8, result);
        }

        let sql = "SELECT * FROM encrypted WHERE encrypted_float8 < $1";
        let rows = client.query(sql, &[&high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result: f64 = row.get("encrypted_float8");
            assert_eq!(encrypted_float8, result);
        }

        let sql = "SELECT * FROM encrypted WHERE encrypted_float8 > $1 and encrypted_float8 < $2 ";
        let rows = client.query(sql, &[&low, &high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result: f64 = row.get("encrypted_float8");
            assert_eq!(encrypted_float8, result);
        }
    }

    #[tokio::test]
    async fn map_ore_where_date() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_date = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();
        let low = NaiveDate::parse_from_str("2024-01-01", "%Y-%m-%d").unwrap();
        let high = NaiveDate::parse_from_str("2027-01-01", "%Y-%m-%d").unwrap();

        let sql = "INSERT INTO encrypted (id, encrypted_date) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_date])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_date FROM encrypted WHERE encrypted_date > $1";
        let rows = client.query(sql, &[&low]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result: NaiveDate = row.get("encrypted_date");
            assert_eq!(encrypted_date, result);
        }

        let sql = "SELECT id, encrypted_date FROM encrypted WHERE encrypted_date < $1";
        let rows = client.query(sql, &[&high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result: NaiveDate = row.get("encrypted_date");
            assert_eq!(encrypted_date, result);
        }

        let sql = "SELECT id, encrypted_date FROM encrypted WHERE encrypted_date > $1 AND encrypted_date < $2";
        let rows = client.query(sql, &[&low, &high]).await.expect("ok");

        assert!(rows.len() == 1);
        for row in rows {
            let result: NaiveDate = row.get("encrypted_date");
            assert_eq!(encrypted_date, result);
        }
    }

    #[tokio::test]
    async fn map_ore_where_text() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "ABC";
        let search_text = "XYZ";

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_text])
            .await
            .expect("ok");

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE encrypted_text < $1";
        let rows = client.query(sql, &[&search_text]).await.expect("ok");

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }
    }
}
