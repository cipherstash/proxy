#[cfg(test)]
mod tests {
    use tokio_postgres::SimpleQueryMessage;

    use crate::common::{clear, connect_with_tls, id, trace, PROXY};

    #[tokio::test]
    async fn map_ore_order_text() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let s_one = "a";
        let s_two = "b";
        let s_three = "c";

        let sql = "
            INSERT INTO encrypted (id, encrypted_text)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(sql, &[&id(), &s_two, &id(), &s_one, &id(), &s_three])
            .await
            .unwrap();

        let sql = "SELECT encrypted_text FROM encrypted ORDER BY encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
        let expected = vec![s_one, s_two, s_three];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_text_desc() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let s_one = "a";
        let s_two = "b";
        let s_three = "c";

        let sql = "
            INSERT INTO encrypted (id, encrypted_text)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(sql, &[&id(), &s_two, &id(), &s_one, &id(), &s_three])
            .await
            .unwrap();

        let sql = "SELECT encrypted_text FROM encrypted ORDER BY encrypted_text DESC";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
        let expected = vec![s_three, s_two, s_one];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_nulls_last_by_default() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let s_one = "a";
        let s_two = "b";

        client
            .query("INSERT INTO encrypted (id) values ($1)", &[&id()])
            .await
            .unwrap();

        let sql = "
            INSERT INTO encrypted (id, encrypted_text)
            VALUES ($1, $2), ($3, $4)
        ";

        client
            .query(sql, &[&id(), &s_one, &id(), &s_two])
            .await
            .unwrap();

        let sql = "SELECT encrypted_text FROM encrypted ORDER BY encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows
            .iter()
            .map(|row| row.get(0))
            .collect::<Vec<Option<String>>>();
        let expected = vec![Some(s_one.to_string()), Some(s_two.to_string()), None];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_nulls_first() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let s_one = "a";
        let s_two = "b";

        let sql = "
            INSERT INTO encrypted (id, encrypted_text)
            VALUES ($1, $2), ($3, $4)
        ";

        client
            .query(sql, &[&id(), &s_one, &id(), &s_two])
            .await
            .unwrap();

        client
            .query("INSERT INTO encrypted (id) values ($1)", &[&id()])
            .await
            .unwrap();

        let sql = "SELECT encrypted_text FROM encrypted ORDER BY encrypted_text NULLS FIRST";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows
            .iter()
            .map(|row| row.get(0))
            .collect::<Vec<Option<String>>>();
        let expected = vec![None, Some(s_one.to_string()), Some(s_two.to_string())];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_qualified_column() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let s_one = "a";
        let s_two = "b";
        let s_three = "c";

        let sql = "
            INSERT INTO encrypted (id, encrypted_text)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(sql, &[&id(), &s_two, &id(), &s_one, &id(), &s_three])
            .await
            .unwrap();

        let sql = "SELECT encrypted_text FROM encrypted ORDER BY encrypted.encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
        let expected = vec![s_one, s_two, s_three];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_qualified_column_with_alias() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let s_one = "a";
        let s_two = "b";
        let s_three = "c";

        let sql = "
            INSERT INTO encrypted (id, encrypted_text)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(sql, &[&id(), &s_two, &id(), &s_one, &id(), &s_three])
            .await
            .unwrap();

        let sql = "SELECT encrypted_text FROM encrypted e ORDER BY e.encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
        let expected = vec![s_one, s_two, s_three];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_simple_protocol() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let sql = format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({}, 'y'), ({}, 'x'), ({}, 'z')",
            id(),
            id(),
            id()
        );

        client.simple_query(&sql).await.unwrap();

        let sql = "SELECT encrypted_text FROM encrypted ORDER BY encrypted_text";
        let rows = client.simple_query(sql).await.unwrap();

        let actual = rows
            .iter()
            .filter_map(|row| {
                if let SimpleQueryMessage::Row(row) = row {
                    row.get(0)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let expected = vec!["x", "y", "z"];

        assert_eq!(actual, expected);
    }

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
