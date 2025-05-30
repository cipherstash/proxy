#[cfg(test)]
mod tests {
    use tokio_postgres::SimpleQueryMessage;

    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};

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
            .query(
                sql,
                &[
                    &random_id(),
                    &s_two,
                    &random_id(),
                    &s_one,
                    &random_id(),
                    &s_three,
                ],
            )
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
            .query(
                sql,
                &[
                    &random_id(),
                    &s_two,
                    &random_id(),
                    &s_one,
                    &random_id(),
                    &s_three,
                ],
            )
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
            .query("INSERT INTO encrypted (id) values ($1)", &[&random_id()])
            .await
            .unwrap();

        let sql = "
            INSERT INTO encrypted (id, encrypted_text)
            VALUES ($1, $2), ($3, $4)
        ";

        client
            .query(sql, &[&random_id(), &s_one, &random_id(), &s_two])
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
            .query(sql, &[&random_id(), &s_one, &random_id(), &s_two])
            .await
            .unwrap();

        client
            .query("INSERT INTO encrypted (id) values ($1)", &[&random_id()])
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
            .query(
                sql,
                &[
                    &random_id(),
                    &s_two,
                    &random_id(),
                    &s_one,
                    &random_id(),
                    &s_three,
                ],
            )
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
            .query(
                sql,
                &[
                    &random_id(),
                    &s_two,
                    &random_id(),
                    &s_one,
                    &random_id(),
                    &s_three,
                ],
            )
            .await
            .unwrap();

        let sql = "SELECT encrypted_text FROM encrypted e ORDER BY e.encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
        let expected = vec![s_one, s_two, s_three];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_no_eql_column_in_select_projection() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id_one = random_id();
        let s_one = "a";
        let id_two = random_id();
        let s_two = "b";
        let id_three = random_id();
        let s_three = "c";

        let sql = "
            INSERT INTO encrypted (id, encrypted_text)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(
                sql,
                &[&id_two, &s_two, &id_one, &s_one, &id_three, &s_three],
            )
            .await
            .unwrap();

        let sql = "SELECT id FROM encrypted ORDER BY encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<i64>>();
        let expected = vec![id_one, id_two, id_three];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn can_order_by_plaintext_column() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let s_one = "a";
        let s_two = "b";
        let s_three = "c";

        let sql = "
            INSERT INTO encrypted (id, plaintext)
            VALUES ($1, $2), ($3, $4), ($5, $6)
        ";

        client
            .query(
                sql,
                &[
                    &random_id(),
                    &s_two,
                    &random_id(),
                    &s_one,
                    &random_id(),
                    &s_three,
                ],
            )
            .await
            .unwrap();

        let sql = "SELECT plaintext FROM encrypted ORDER BY plaintext";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
        let expected = vec![s_one, s_two, s_three];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn can_order_by_plaintext_and_eql_columns() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let s_plaintext_one = "a";
        let s_plaintext_two = "a";
        let s_plaintext_three = "b";

        let s_enctrypted_one = "a";
        let s_encrypted_two = "b";
        let s_encrypted_three = "c";

        let sql = "
            INSERT INTO encrypted (id, plaintext, encrypted_text)
            VALUES ($1, $2, $3), ($4, $5, $6), ($7, $8, $9)
        ";

        client
            .query(
                sql,
                &[
                    &random_id(),
                    &s_plaintext_two,
                    &s_encrypted_two,
                    &random_id(),
                    &s_plaintext_one,
                    &s_enctrypted_one,
                    &random_id(),
                    &s_plaintext_three,
                    &s_encrypted_three,
                ],
            )
            .await
            .unwrap();

        let sql =
            "SELECT plaintext, encrypted_text FROM encrypted ORDER BY plaintext, encrypted_text";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows
            .iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect::<Vec<(&str, &str)>>();

        let expected = vec![
            (s_plaintext_one, s_enctrypted_one),
            (s_plaintext_two, s_encrypted_two),
            (s_plaintext_three, s_encrypted_three),
        ];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ore_order_simple_protocol() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let sql = format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({}, 'y'), ({}, 'x'), ({}, 'z')",
            random_id(),
            random_id(),
            random_id()
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
            .query(
                sql,
                &[
                    &random_id(),
                    &n_two,
                    &random_id(),
                    &n_one,
                    &random_id(),
                    &n_three,
                ],
            )
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
            .query(
                sql,
                &[
                    &random_id(),
                    &n_two,
                    &random_id(),
                    &n_one,
                    &random_id(),
                    &n_three,
                ],
            )
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
            .query(
                sql,
                &[
                    &random_id(),
                    &n_two,
                    &random_id(),
                    &n_one,
                    &random_id(),
                    &n_three,
                ],
            )
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
            .query(
                sql,
                &[
                    &random_id(),
                    &n_two,
                    &random_id(),
                    &n_one,
                    &random_id(),
                    &n_three,
                ],
            )
            .await
            .unwrap();

        let sql = "SELECT encrypted_int8 FROM encrypted ORDER BY encrypted_int8";
        let rows = client.query(sql, &[]).await.unwrap();

        let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<i64>>();
        let expected = vec![n_one, n_two, n_three];

        assert_eq!(actual, expected);
    }
}
