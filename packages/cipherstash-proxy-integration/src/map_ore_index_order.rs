#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};

    // TODO: tests for text fields

    #[tokio::test]
    async fn map_ore_order_int2() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id_one = id();
        let n_one = 10i16;
        let id_two = id();
        let n_two = 20i16;
        let id_three = id();
        let n_three = 30i16;

        let sql = "
            INSERT INTO encrypted (id, encrypted_int2)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(
                sql,
                &[&id_two, &n_two, &id_one, &n_one, &id_three, &n_three],
            )
            .await
            .unwrap();

        let sql = "SELECT encrypted_int2 FROM encrypted ORDER BY encrypted_int2 ASC";
        let rows = client.query(sql, &[]).await.unwrap();

        assert_eq!(rows.len(), 3);

        for row in &rows {
            let i: i16 = row.get("encrypted_int2");
            dbg!(i);
        }

        let row = &rows[0];
        let result_int: i16 = row.get("encrypted_int2");
        assert_eq!(n_one, result_int);

        let row = &rows[1];
        let result_int: i16 = row.get("encrypted_int2");
        assert_eq!(n_two, result_int);

        let row = &rows[2];
        let result_int: i16 = row.get("encrypted_int2");
        assert_eq!(n_three, result_int);
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
