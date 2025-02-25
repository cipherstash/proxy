#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};

    // TODO: tests for text fields

    #[tokio::test]
    async fn map_ore_order_int2() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let n_one = 10i16;
        let n_two = 20i16;
        let n_three = 30i16;

        let sql = "
            INSERT INTO encrypted (id, encrypted_int2)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(sql, &[&id(), &n_two, &id(), &n_one, &id(), &n_three])
            .await
            .unwrap();

        let sql = "SELECT encrypted_int2 FROM encrypted ORDER BY encrypted_int2";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<i16>>();
        let expected = vec![n_one, n_two, n_three];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_int2_desc() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let n_one = 10i16;
        let n_two = 20i16;
        let n_three = 30i16;

        let sql = "
            INSERT INTO encrypted (id, encrypted_int2)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(sql, &[&id(), &n_two, &id(), &n_one, &id(), &n_three])
            .await
            .unwrap();

        let sql = "SELECT encrypted_int2 FROM encrypted ORDER BY encrypted_int2 DESC";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<i16>>();
        let expected = vec![n_three, n_two, n_one];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_int4() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let n_one = 10i32;
        let n_two = 20i32;
        let n_three = 30i32;

        let sql = "
            INSERT INTO encrypted (id, encrypted_int4)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(sql, &[&id(), &n_two, &id(), &n_one, &id(), &n_three])
            .await
            .unwrap();

        let sql = "SELECT encrypted_int4 FROM encrypted ORDER BY encrypted_int4";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<i32>>();
        let expected = vec![n_one, n_two, n_three];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_int8() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let n_one = 10i64;
        let n_two = 20i64;
        let n_three = 30i64;

        let sql = "
            INSERT INTO encrypted (id, encrypted_int8)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(sql, &[&id(), &n_two, &id(), &n_one, &id(), &n_three])
            .await
            .unwrap();

        let sql = "SELECT encrypted_int8 FROM encrypted ORDER BY encrypted_int8";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<i64>>();
        let expected = vec![n_one, n_two, n_three];

        assert_eq!(actual, expected);
    }
}
