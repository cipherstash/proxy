#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};

    // Currently not working, needs statement rewriting
    // #[tokio::test]
    async fn _map_ore_order_int2() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let low_id = id();
        let high_id = id();
        let low: i16 = 1;
        let high: i16 = 99;

        let sql = "INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)";
        client.query(sql, &[&low_id, &low]).await.unwrap();

        let sql = "INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)";
        client.query(sql, &[&high_id, &high]).await.unwrap();

        let sql = "SELECT encrypted_int2 FROM encrypted ORDER BY encrypted_int2 ASC";
        let rows = client.query(sql, &[]).await.unwrap();

        assert_eq!(rows.len(), 2);

        let row = &rows[0];
        let result_int: i16 = row.get("encrypted_int2");
        assert_eq!(low, result_int);

        let row = &rows[1];
        let result_int: i16 = row.get("encrypted_int2");
        assert_eq!(high, result_int);
    }

    // Currently not working, needs statement rewriting
    // #[tokio::test]
    async fn _map_ore_order_int4() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let low_id = id();
        let high_id = id();
        let low: i32 = 1;
        let high: i32 = 99;

        let sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        client.query(sql, &[&low_id, &low]).await.unwrap();

        let sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        client.query(sql, &[&high_id, &high]).await.unwrap();

        let sql = "SELECT encrypted_int4 FROM encrypted ORDER BY encrypted_int4 ASC";
        let rows = client.query(sql, &[]).await.unwrap();

        assert_eq!(rows.len(), 2);

        let row = &rows[0];
        let result_int: i32 = row.get("encrypted_int4");
        assert_eq!(low, result_int);

        let row = &rows[1];
        let result_int: i32 = row.get("encrypted_int4");
        assert_eq!(high, result_int);
    }

    // Currently not working, needs statement rewriting
    // #[tokio::test]
    async fn _map_ore_order_int8() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let low_id = id();
        let high_id = id();
        let low: i64 = 1;
        let high: i64 = 99;

        let sql = "INSERT INTO encrypted (id, encrypted_int8) VALUES ($1, $2)";
        client.query(sql, &[&low_id, &low]).await.unwrap();

        let sql = "INSERT INTO encrypted (id, encrypted_int8) VALUES ($1, $2)";
        client.query(sql, &[&high_id, &high]).await.unwrap();

        let sql = "SELECT encrypted_int8 FROM encrypted ORDER BY encrypted_int8 ASC";
        let rows = client.query(sql, &[]).await.unwrap();

        assert_eq!(rows.len(), 2);

        let row = &rows[0];
        let result_int: i64 = row.get("encrypted_int8");
        assert_eq!(low, result_int);

        let row = &rows[1];
        let result_int: i64 = row.get("encrypted_int8");
        assert_eq!(high, result_int);
    }
}
