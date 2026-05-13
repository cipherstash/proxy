#[cfg(test)]
mod tests {
    use crate::common::{
        clear_table, connect_with_tls, interleaved_indices, random_id, trace, PROXY,
    };
    use std::fmt::Debug;
    use tokio_postgres::types::{FromSql, ToSql};

    #[tokio::test]
    async fn map_ope_order_text_asc() {
        run_order_test(
            "encrypted_ope_order_text_asc",
            "encrypted_text",
            text_values(),
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ope_order_text_desc() {
        run_order_test(
            "encrypted_ope_order_text_desc",
            "encrypted_text",
            text_values(),
            true,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ope_order_int4_asc() {
        run_order_test(
            "encrypted_ope_order_int4_asc",
            "encrypted_int4",
            vec![-100i32, -1, 0, 1, 42, 1000, i32::MAX],
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ope_order_int4_desc() {
        run_order_test(
            "encrypted_ope_order_int4_desc",
            "encrypted_int4",
            vec![-100i32, -1, 0, 1, 42, 1000, i32::MAX],
            true,
        )
        .await;
    }

    fn text_values() -> Vec<String> {
        ["aardvark", "aplomb", "chimera", "chrysalis", "zephyr"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Inserts `values` (given in ascending order) in interleaved order, then
    /// verifies `ORDER BY {col_name}` returns them sorted in the requested
    /// direction.
    async fn run_order_test<T>(table: &str, col_name: &str, values: Vec<T>, descending: bool)
    where
        for<'a> T: PartialEq + ToSql + Sync + FromSql<'a> + Debug,
    {
        trace();
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let insert = format!("INSERT INTO {table} (id, {col_name}) VALUES ($1, $2)");
        for idx in interleaved_indices(values.len()) {
            client
                .query(&insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let dir = if descending { "DESC" } else { "ASC" };
        let select = format!("SELECT {col_name} FROM {table} ORDER BY {col_name} {dir}");
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<T> = rows.iter().map(|r| r.get(0)).collect();
        let expected: Vec<T> = if descending {
            values.into_iter().rev().collect()
        } else {
            values
        };
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ope_order_nulls_last_by_default() {
        trace();
        let table = "encrypted_ope_order_nulls_last";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let null_insert = format!("INSERT INTO {table} (id) VALUES ($1)");
        client.query(&null_insert, &[&random_id()]).await.unwrap();

        let insert = format!("INSERT INTO {table} (id, encrypted_text) VALUES ($1, $2), ($3, $4)");
        client
            .query(&insert, &[&random_id(), &"a", &random_id(), &"b"])
            .await
            .unwrap();

        let select = format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text");
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<Option<String>> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(
            actual,
            vec![Some("a".into()), Some("b".into()), None],
            "NULLs should sort last by default"
        );
    }

    #[tokio::test]
    async fn map_ope_order_nulls_first() {
        trace();
        let table = "encrypted_ope_order_nulls_first";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let insert = format!("INSERT INTO {table} (id, encrypted_text) VALUES ($1, $2), ($3, $4)");
        client
            .query(&insert, &[&random_id(), &"a", &random_id(), &"b"])
            .await
            .unwrap();

        let null_insert = format!("INSERT INTO {table} (id) VALUES ($1)");
        client.query(&null_insert, &[&random_id()]).await.unwrap();

        let select =
            format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text NULLS FIRST");
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<Option<String>> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(
            actual,
            vec![None, Some("a".into()), Some("b".into())],
            "NULLS FIRST should explicitly sort NULLs first"
        );
    }
}
