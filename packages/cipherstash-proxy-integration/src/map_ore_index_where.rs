#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use chrono::NaiveDate;
    use tokio_postgres::types::{FromSql, ToSql};
    use tokio_postgres::Client;

    #[tokio::test]
    async fn map_ore_where_generic_int2() {
        map_ore_where_generic("encrypted_int2", 40i16, 99i16).await;
    }

    #[tokio::test]
    async fn map_ore_where_generic_int4() {
        map_ore_where_generic("encrypted_int4", 40i32, 99i32).await;
    }

    #[tokio::test]
    async fn map_ore_where_generic_int8() {
        map_ore_where_generic("encrypted_int8", 40i64, 99i64).await;
    }

    #[tokio::test]
    async fn map_ore_where_generic_float8() {
        map_ore_where_generic("encrypted_float8", 40.0f64, 99.0f64).await;
    }

    #[tokio::test]
    async fn map_ore_where_generic_date() {
        let low = NaiveDate::parse_from_str("2024-01-01", "%Y-%m-%d").unwrap();
        let high = NaiveDate::parse_from_str("2027-01-01", "%Y-%m-%d").unwrap();
        map_ore_where_generic("encrypted_date", low, high).await;
    }

    #[tokio::test]
    async fn map_ore_where_generic_text() {
        map_ore_where_generic("encrypted_text", "ABC".to_string(), "BCD".to_string()).await;
    }

    #[tokio::test]
    async fn map_ore_where_generic_bool() {
        map_ore_where_generic("encrypted_bool", false, true).await;
    }

    /// Tests ORE operations with 2 values - high & low.
    /// The type of column identified by col_name must match the parameters
    /// such as INT2 and i16, FLOAT8 and f64
    async fn map_ore_where_generic<T>(col_name: &str, low: T, high: T)
    where
        for<'a> T: Clone + PartialEq + ToSql + Sync + FromSql<'a> + PartialOrd,
    {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        // Insert test data
        let sql = format!("INSERT INTO encrypted (id, {col_name}) VALUES ($1, $2)");
        for val in [low.clone(), high.clone()] {
            client
                .query(&sql, &[&id(), &val])
                .await
                .expect("insert failed");
        }

        /* Currently, the `<` operator returns null rows when they shouldn't
         * TODO: uncomment this when the operator issue is addressed
        let sql = format!("INSERT INTO encrypted (id, {col_name}) VALUES ($1, null)");
        client
            .query(&sql, &[&id()])
            .await
            .expect("insert failed");
         */

        // GT: given [1, 3], `> 1` returns [3]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} > $1");
        test_ore_op(&client, col_name, &sql, &[&low], &[high.clone()]).await;

        // LT: given [1, 3], `< 3` returns [1]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} < $1");
        test_ore_op(&client, col_name, &sql, &[&high], &[low.clone()]).await;

        // GT && LT: given [1, 3], `> 1 and < 3` returns []
        let sql =
            format!("SELECT {col_name} FROM encrypted WHERE {col_name} > $1 AND {col_name} < $2");
        test_ore_op::<T>(&client, col_name, &sql, &[&low, &high], &[]).await;

        // LTEQ: given [1, 3], `<= 3` returns [1, 3]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} <= $1");
        test_ore_op(
            &client,
            col_name,
            &sql,
            &[&high],
            &[low.clone(), high.clone()],
        )
        .await;

        // GTEQ: given [1, 3], `>= 1` returns [1, 3]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} >= $1");
        test_ore_op(
            &client,
            col_name,
            &sql,
            &[&low],
            &[low.clone(), high.clone()],
        )
        .await;

        // EQ: given [1, 3], `= 1` returns [1]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} = $1");
        test_ore_op(&client, col_name, &sql, &[&low], &[low.clone()]).await;

        // NEQ: given [1, 3], `<> 3` returns [1]
        let sql = format!("SELECT {col_name} FROM encrypted WHERE {col_name} <> $1");
        test_ore_op(&client, col_name, &sql, &[&high], &[low.clone()]).await;
    }

    /// Runs the query and checks the returned results match the expected results.
    /// The results are sorted after the query as there are separate tests for ordering
    /// Using sort_by & partial_cmp here because this is used for floats too (NaN cannot be compared)
    async fn test_ore_op<T>(
        client: &Client,
        col_name: &str,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        expected: &[T],
    ) where
        for<'a> T: ToSql + PartialEq + Sync + FromSql<'a> + PartialOrd,
    {
        let rows = client.query(sql, params).await.expect("query failed");

        let mut results: Vec<_> = rows
            .iter()
            .map(|r| r.get::<&str, T>(col_name))
            .collect::<Vec<_>>();
        results.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(expected, &results);
    }
}
