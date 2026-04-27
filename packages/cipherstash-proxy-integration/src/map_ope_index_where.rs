#[cfg(test)]
mod tests {
    use crate::common::{clear_table, connect_with_tls, random_id, trace, PROXY};
    use chrono::NaiveDate;
    use tokio_postgres::types::{FromSql, ToSql};
    use tokio_postgres::Client;

    #[tokio::test]
    async fn map_ope_where_generic_int2() {
        map_ope_where_generic(
            "encrypted_ope_where_int2",
            "encrypted_int2",
            40i16,
            99i16,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ope_where_generic_int4() {
        map_ope_where_generic(
            "encrypted_ope_where_int4",
            "encrypted_int4",
            40i32,
            99i32,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ope_where_generic_int8() {
        map_ope_where_generic(
            "encrypted_ope_where_int8",
            "encrypted_int8",
            40i64,
            99i64,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ope_where_generic_float8() {
        map_ope_where_generic(
            "encrypted_ope_where_float8",
            "encrypted_float8",
            40.0f64,
            99.0f64,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ope_where_generic_date() {
        let low = NaiveDate::parse_from_str("2024-01-01", "%Y-%m-%d").unwrap();
        let high = NaiveDate::parse_from_str("2027-01-01", "%Y-%m-%d").unwrap();
        map_ope_where_generic("encrypted_ope_where_date", "encrypted_date", low, high).await;
    }

    #[tokio::test]
    async fn map_ope_where_generic_text() {
        map_ope_where_generic(
            "encrypted_ope_where_text",
            "encrypted_text",
            "ABC".to_string(),
            "BCD".to_string(),
        )
        .await;
    }

    #[tokio::test]
    async fn map_ope_where_generic_bool() {
        map_ope_where_generic(
            "encrypted_ope_where_bool",
            "encrypted_bool",
            false,
            true,
        )
        .await;
    }

    /// Tests OPE operations against a per-test fixture table.
    /// Mirrors `map_ore_where_generic` but targets the OPE-indexed mirror tables.
    async fn map_ope_where_generic<T>(table: &str, col_name: &str, low: T, high: T)
    where
        for<'a> T: Clone + ToSql + PartialEq + Sync + FromSql<'a> + PartialOrd,
    {
        trace();

        clear_table(table).await;

        let client = connect_with_tls(PROXY).await;

        // Insert test data
        let sql = format!("INSERT INTO {table} (id, {col_name}) VALUES ($1, $2)");
        for val in [low.clone(), high.clone()] {
            client
                .query(&sql, &[&random_id(), &val])
                .await
                .expect("insert failed");
        }

        // NULL record
        let sql = format!("INSERT INTO {table} (id, {col_name}) VALUES ($1, null)");
        client
            .query(&sql, &[&random_id()])
            .await
            .expect("insert failed");

        // GT: given [1, 3], `> 1` returns [3]
        let sql = format!("SELECT {col_name} FROM {table} WHERE {col_name} > $1");
        test_ope_op(
            &client,
            col_name,
            &sql,
            &[&low],
            std::slice::from_ref(&high),
        )
        .await;

        // GT 2nd case: given [1, 3], `> 3` returns []
        let sql = format!("SELECT {col_name} FROM {table} WHERE {col_name} > $1");
        test_ope_op::<T>(&client, col_name, &sql, &[&high], &[]).await;

        // LT: given [1, 3], `< 3` returns [1]
        let sql = format!("SELECT {col_name} FROM {table} WHERE {col_name} < $1");
        test_ope_op(
            &client,
            col_name,
            &sql,
            &[&high],
            std::slice::from_ref(&low),
        )
        .await;

        // LT 2nd case: given [1, 3], `< 1` returns []
        let sql = format!("SELECT {col_name} FROM {table} WHERE {col_name} < $1");
        test_ope_op(&client, col_name, &sql, &[&low], &[] as &[T]).await;

        // GT && LT: given [1, 3], `> 1 and < 3` returns []
        let sql =
            format!("SELECT {col_name} FROM {table} WHERE {col_name} > $1 AND {col_name} < $2");
        test_ope_op(&client, col_name, &sql, &[&low, &high], &[] as &[T]).await;

        // LTEQ: given [1, 3], `<= 3` returns [1, 3]
        let sql = format!("SELECT {col_name} FROM {table} WHERE {col_name} <= $1");
        test_ope_op(
            &client,
            col_name,
            &sql,
            &[&high],
            &[low.clone(), high.clone()],
        )
        .await;

        // GTEQ: given [1, 3], `>= 1` returns [1, 3]
        let sql = format!("SELECT {col_name} FROM {table} WHERE {col_name} >= $1");
        test_ope_op(
            &client,
            col_name,
            &sql,
            &[&low],
            &[low.clone(), high.clone()],
        )
        .await;

        // EQ: given [1, 3], `= 1` returns [1]
        let sql = format!("SELECT {col_name} FROM {table} WHERE {col_name} = $1");
        test_ope_op(&client, col_name, &sql, &[&low], std::slice::from_ref(&low)).await;

        // NEQ: given [1, 3], `<> 3` returns [1]
        let sql = format!("SELECT {col_name} FROM {table} WHERE {col_name} <> $1");
        test_ope_op(
            &client,
            col_name,
            &sql,
            &[&high],
            std::slice::from_ref(&low),
        )
        .await;
    }

    /// Runs the query and checks the returned results match the expected results.
    /// Sorts after the query (separate tests cover ordering).
    async fn test_ope_op<T>(
        client: &Client,
        col_name: &str,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        expected: &[T],
    ) where
        for<'a> T: Clone + ToSql + PartialEq + Sync + FromSql<'a> + PartialOrd,
    {
        let rows = client.query(sql, params).await.expect("query failed");
        let mut actual: Vec<T> = rows.iter().map(|r| r.get(0)).collect();
        actual.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut expected: Vec<T> = expected.to_vec();
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());

        assert_eq!(
            actual.len(),
            expected.len(),
            "wrong row count for {col_name} via {sql}"
        );
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert!(a == e, "value mismatch for {col_name} via {sql}");
        }
    }
}
