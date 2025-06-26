#[cfg(test)]
mod tests {
    use crate::common::{clear, execute_query, query, random_id, simple_query, trace};
    use chrono::NaiveDate;

    macro_rules! test_order_by {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;
                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));

                let mut expected = vec![];
                for i in 1..=10 {
                    let encrypted_val = crate::value_for_type!($type, i);

                    let id = random_id();
                    let sql =
                        format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ($1, $2)");
                    execute_query(&sql, &[&id, &encrypted_val]).await;

                    expected.push(encrypted_val);
                }

                let sql =
                    format!("SELECT {encrypted_col} FROM encrypted ORDER BY {encrypted_col} ASC");

                let actual = query::<$type>(&sql).await;

                assert_eq!(expected, actual);

                let actual = simple_query::<$type>(&sql).await;
                assert_eq!(expected, actual);

                let sql =
                    format!("SELECT {encrypted_col} FROM encrypted ORDER BY {encrypted_col} DESC");

                expected.reverse();

                let actual = query::<$type>(&sql).await;
                assert_eq!(expected, actual);

                let actual = simple_query::<$type>(&sql).await;
                assert_eq!(expected, actual);
            }
        };
    }

    test_order_by!(order_by_int2, i16, int2);
    test_order_by!(order_by_int4, i32, int4);
    test_order_by!(order_by_int8, i64, int8);
    test_order_by!(order_by_float8, f64, float8);
    test_order_by!(order_by_text, String, text);
    test_order_by!(order_by_date, NaiveDate, date);

    // Bool breaks the macro logic, will come to figure it out
    // test_order_by!(order_by_bool, bool, bool);
}
