#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use chrono::NaiveDate;
    use tokio_postgres::types::{ToSql, FromSql};
    use tokio_postgres::Client;

    #[tokio::test]
    async fn map_ore_where_generic_int2() {
        map_ore_where_generic("encrypted_int2", 40i16, 42i16, 99i16).await;
    }

    /// Tests ORE operations with 3 values - high, mid & low.
    /// The type of column identified by col_name must match the parameters
    /// such as INT2 and i16, FLOAT8 and f64
    async fn map_ore_where_generic<T>(col_name: &str, low: T, mid: T, high: T)
    where
        for<'a> T: Clone + PartialEq + ToSql + Sync + FromSql<'a> + PartialOrd
    {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let sql = format!("INSERT INTO encrypted (id, {col_name}) VALUES ($1, $2)");

        for val in [low.clone(), mid.clone(), high.clone()] {
            client
                .query(&sql, &[&id(), &val])
                .await
                .expect("insert failed");
        }

        // GT: given [1, 2, 3], `> 2` returns [3]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} > $1");
        test_ore_op(&client, col_name, &sql, &[&mid], &[high.clone()]).await;

        // LT: given [1, 2, 3], `< 2` returns [1]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} < $1");
        test_ore_op(&client, col_name, &sql, &[&mid], &[low.clone()]).await;

        // GT && LT: given [1, 2, 3], `> 1 and < 3` returns [2]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} > $1 AND {col_name} < $2");
        test_ore_op(&client, col_name, &sql, &[&low, &high], &[mid.clone()]).await;

        // LTEQ: given [1, 2, 3], `<= 2` returns [1, 2]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} <= $1");
        test_ore_op(&client, col_name, &sql, &[&mid], &[low.clone(), mid.clone()]).await;

        // GTEQ: given [1, 2, 3], `>= 2` returns [2, 3]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} >= $1");
        test_ore_op(&client, col_name, &sql, &[&mid], &[mid.clone(), high.clone()]).await;

        // EQ: given [1, 2, 3], `= 2` returns [2]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} = $1");
        test_ore_op(&client, col_name, &sql, &[&mid], &[mid.clone()]).await;

        // NEQ: given [1, 2, 3], `<> 2` returns [1, 3]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} <> $1");
        test_ore_op(&client, col_name, &sql, &[&mid], &[low.clone(), high.clone()]).await;
    }

    /// Runs the query and checks the returned results match the expected results.
    /// The results are sorted after the query as there are separate tests for ordering
    /// Using sort_by & partial_cmp here because this is used for floats too (NaN cannot be compared)
    async fn test_ore_op<T>(client: &Client, col_name: &str, sql: &str, params: &[&(dyn ToSql + Sync)], expected: &[T])
    where
        for<'a> T: ToSql + PartialEq + Sync + FromSql<'a> + PartialOrd
    {
        let rows = client.query(sql, params).await.expect("query failed");

        let mut results: Vec<_> = rows
            .iter()
            .map(|r| r.get::<&str, T>(&format!("{col_name}")))
            .collect::<Vec<_>>();
        results.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(expected, &results);
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
