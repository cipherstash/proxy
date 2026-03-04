#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use tokio_postgres::types::{FromSql, ToSql};
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
        let values: Vec<i16> = vec![-100, -10, -1, 0, 1, 5, 10, 20, 100, 200];
        map_ore_order_generic("encrypted_int2", values, "ASC").await;
    }

    #[tokio::test]
    async fn map_ore_order_int2_desc() {
        let values: Vec<i16> = vec![-100, -10, -1, 0, 1, 5, 10, 20, 100, 200];
        map_ore_order_generic("encrypted_int2", values, "DESC").await;
    }

    #[tokio::test]
    async fn map_ore_order_int4() {
        let values: Vec<i32> = vec![-50_000, -1_000, -1, 0, 1, 42, 1_000, 10_000, 50_000, 100_000];
        map_ore_order_generic("encrypted_int4", values, "ASC").await;
    }

    #[tokio::test]
    async fn map_ore_order_int4_desc() {
        let values: Vec<i32> = vec![-50_000, -1_000, -1, 0, 1, 42, 1_000, 10_000, 50_000, 100_000];
        map_ore_order_generic("encrypted_int4", values, "DESC").await;
    }

    #[tokio::test]
    async fn map_ore_order_int8() {
        let values: Vec<i64> = vec![-1_000_000, -10_000, -1, 0, 1, 42, 10_000, 100_000, 1_000_000, 9_999_999];
        map_ore_order_generic("encrypted_int8", values, "ASC").await;
    }

    #[tokio::test]
    async fn map_ore_order_int8_desc() {
        let values: Vec<i64> = vec![-1_000_000, -10_000, -1, 0, 1, 42, 10_000, 100_000, 1_000_000, 9_999_999];
        map_ore_order_generic("encrypted_int8", values, "DESC").await;
    }

    #[tokio::test]
    async fn map_ore_order_float8() {
        let values: Vec<f64> = vec![-99.9, -1.5, -0.001, 0.0, 0.001, 1.5, 3.14, 42.0, 99.9, 1000.5];
        map_ore_order_generic("encrypted_float8", values, "ASC").await;
    }

    #[tokio::test]
    async fn map_ore_order_float8_desc() {
        let values: Vec<f64> = vec![-99.9, -1.5, -0.001, 0.0, 0.001, 1.5, 3.14, 42.0, 99.9, 1000.5];
        map_ore_order_generic("encrypted_float8", values, "DESC").await;
    }

    /// Returns indices in zigzag order so insertion is never accidentally sorted.
    /// For len=5: [4, 0, 3, 1, 2]
    fn interleaved_indices(len: usize) -> Vec<usize> {
        let mut indices = Vec::with_capacity(len);
        let mut lo = 0;
        let mut hi = len;
        let mut take_hi = true;
        while lo < hi {
            if take_hi {
                hi -= 1;
                indices.push(hi);
            } else {
                indices.push(lo);
                lo += 1;
            }
            take_hi = !take_hi;
        }
        indices
    }

    /// Generic ORE ordering test.
    ///
    /// `values` must be provided in ascending sorted order.
    /// Values are inserted in interleaved (non-sorted) order, then verified
    /// via ORDER BY in the given direction.
    async fn map_ore_order_generic<T>(col_name: &str, values: Vec<T>, direction: &str)
    where
        for<'a> T: Clone + PartialEq + ToSql + Sync + FromSql<'a> + PartialOrd + Debug,
    {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let insert_sql = format!("INSERT INTO encrypted (id, {col_name}) VALUES ($1, $2)");

        // Insert in interleaved order to avoid accidentally-sorted insertion
        for idx in interleaved_indices(values.len()) {
            client
                .query(&insert_sql, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let select_sql = format!(
            "SELECT {col_name} FROM encrypted ORDER BY {col_name} {direction}"
        );
        let rows = client.query(&select_sql, &[]).await.unwrap();

        let actual: Vec<T> = rows.iter().map(|row| row.get(0)).collect();

        let expected: Vec<T> = if direction == "DESC" {
            values.into_iter().rev().collect()
        } else {
            values
        };

        assert_eq!(actual, expected);
    }
}
