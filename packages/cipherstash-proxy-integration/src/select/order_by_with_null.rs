#[cfg(test)]
mod tests {
    use crate::common::{clear, execute_query, query, random_id, simple_query_with_null, trace};
    use chrono::NaiveDate;
    use tap::prelude::*;
    use tokio_postgres::types::ToSql;

    async fn insert_encrypted_value<T>(col: &str, val: &T)
    where
        T: ToSql + Sync + Send + 'static,
    {
        let id = random_id();
        let sql = format!("INSERT INTO encrypted (id, {}) VALUES ($1, $2)", col);
        execute_query(&sql, &[&id, &val]).await;
    }

    fn assert_expected<T>(expected: &[Option<T>], actual: &[Option<T>])
    where
        T: std::fmt::Display + PartialEq + std::fmt::Debug,
    {
        assert_eq!(expected.len(), actual.len());
        for (e, r) in expected.iter().zip(actual) {
            // info!("Expected: {:?}, Actual: {:?}", e, r);
            assert_eq!(e, r);
        }
    }

    fn assert_expected_as_string<T>(expected: &[Option<T>], actual: &[Option<String>])
    where
        T: std::fmt::Display + PartialEq + std::fmt::Debug,
    {
        assert_eq!(expected.len(), actual.len());

        for (e, r) in expected.iter().zip(actual) {
            let e_str = e.as_ref().map(|v| v.to_string());
            assert_eq!(e_str.as_ref(), r.as_ref());
        }
    }

    macro_rules! insert_encrypted_values {
        ($type:ident, $encrypted_col:expr) => {{
            let mut expected = vec![];
            for i in 1..=5 {
                let value = crate::value_for_type!($type, i);
                insert_encrypted_value($encrypted_col, &value).await;
                expected.push(value);
            }
            expected
        }};
    }

    // ------------------------------------------------------------------------
    // ASC
    // Default if unspecified is NULLS LAST
    // [Some("A"), Some("B"), Some("C"), Some("D"), Some("E"),None, ]
    macro_rules! test_order_by_with_null_asc {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();
                clear().await;
                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));

                // insert test values
                let expected = insert_encrypted_values!($type, &encrypted_col);

                // insert a null value (after the test values to avoid any natural ordering issues)
                let encrypted_value: Option<$type> = None;
                insert_encrypted_value(&encrypted_col, &encrypted_value).await;

                // NULL values are Option<T> in tokio_postgres
                // Wrap expected values in Some
                // [Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), None, ]
                let expected = expected
                    .into_iter()
                    .map(|v| Some(v))
                    .collect::<Vec<_>>()
                    .tap_mut(|e| {
                        e.push(None);
                    });

                // [Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), None,]
                let sql =
                    format!("SELECT {encrypted_col} FROM encrypted ORDER BY {encrypted_col} ASC");

                let result = query::<Option<$type>>(&sql).await;
                assert_expected(&expected, &result);

                let result = simple_query_with_null(&sql).await;
                assert_expected_as_string(&expected, &result);
            }
        };
    }

    // ------------------------------------------------------------------------
    // ASC NULLS FIRST
    // [None, Some("A"), Some("B"), Some("C"), Some("D"), Some("E")]
    macro_rules! test_order_by_with_null_asc_nulls_first {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                clear().await;
                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));

                // insert test values
                let expected = insert_encrypted_values!($type, &encrypted_col);

                // insert a null value (after the test values to avoid any natural ordering issues)
                let encrypted_value: Option<$type> = None;
                insert_encrypted_value(&encrypted_col, &encrypted_value).await;

                // NULL values are Option<T> in tokio_postgres
                // Wrap expected values in Some
                // [None, Some("A"), Some("B"), Some("C"), Some("D"), Some("E")]
                let expected = expected.into_iter().map(|v| Some(v)).collect::<Vec<_>>().tap_mut(|e| {
                    e.insert(0, None);
                });

                // ------------------------------------------------------------------------
                // ASC NULLS FIRST
                // [None, Some("A"), Some("B"), Some("C"), Some("D"), Some("E")]
                let sql =
                    format!("SELECT {encrypted_col} FROM encrypted ORDER BY {encrypted_col} ASC NULLS FIRST");

                let result = query::<Option<$type>>(&sql).await;
                assert_expected(&expected, &result);

                let result = simple_query_with_null(&sql).await;
                assert_expected_as_string(&expected, &result);

            }
        };
    }

    // ------------------------------------------------------------------------
    // ASC NULLS LAST
    // [Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), None, ]
    macro_rules! test_order_by_with_null_asc_nulls_last {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                clear().await;
                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));

                // insert a null value (before the test values to avoid any natural ordering issues)
                let encrypted_value: Option<$type> = None;
                insert_encrypted_value(&encrypted_col, &encrypted_value).await;

                // insert test values
                let expected = insert_encrypted_values!($type, &encrypted_col);

                // NULL values are Option<T> in tokio_postgres
                // Wrap expected values in Some
                // [Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), None]
                let expected = expected.into_iter().map(|v| Some(v)).collect::<Vec<_>>().tap_mut(|e| {
                    e.push(None);
                });

                // ------------------------------------------------------------------------
                // ASC NULLS LAST
                // [None, Some("A"), Some("B"), Some("C"), Some("D"), Some("E")]
                let sql =
                    format!("SELECT {encrypted_col} FROM encrypted ORDER BY {encrypted_col} ASC NULLS LAST");

                let result = query::<Option<$type>>(&sql).await;
                assert_expected(&expected, &result);

                let result = simple_query_with_null(&sql).await;
                assert_expected_as_string(&expected, &result);

            }
        };
    }

    // ------------------------------------------------------------------------
    // DESC
    // Default if unspecified is NULLS FIRST
    // [None, Some("E"), Some("D"), Some("C"), Some("B"), Some("A")]
    macro_rules! test_order_by_with_null_desc {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();
                clear().await;
                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));

                // insert a null value (before the test values to avoid any natural ordering issues)
                let encrypted_value: Option<$type> = None;
                insert_encrypted_value(&encrypted_col, &encrypted_value).await;

                // insert test values
                let expected = insert_encrypted_values!($type, &encrypted_col);

                // NULL values are Option<T> in tokio_postgres
                // Wrap expected values in Some
                // [None, Some("E"), Some("D"), Some("C"), Some("B"), Some("A")]
                let expected = expected
                    .tap_mut(|e| e.reverse())
                    .into_iter()
                    .map(|v| Some(v))
                    .collect::<Vec<_>>()
                    .tap_mut(|e| {
                        e.insert(0, None);
                    });

                let sql =
                    format!("SELECT {encrypted_col} FROM encrypted ORDER BY {encrypted_col} DESC");

                let result = query::<Option<$type>>(&sql).await;
                assert_expected(&expected, &result);

                let result = simple_query_with_null(&sql).await;
                assert_expected_as_string(&expected, &result);
            }
        };
    }

    // ------------------------------------------------------------------------
    // DESC NULLS FIRST
    // [None, Some("E"), Some("D"), Some("C"), Some("B"), Some("A")]
    macro_rules! test_order_by_with_null_desc_nulls_first {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();
                clear().await;
                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));

                // insert a null value (before the test values to avoid any natural ordering issues)
                let encrypted_value: Option<$type> = None;
                insert_encrypted_value(&encrypted_col, &encrypted_value).await;

                // insert test values
                let expected = insert_encrypted_values!($type, &encrypted_col);

                // NULL values are Option<T> in tokio_postgres
                // Wrap expected values in Some
                // [None, Some("E"), Some("D"), Some("C"), Some("B"), Some("A")]
                let expected = expected.tap_mut(|e| e.reverse()).into_iter().map(|v| Some(v)).collect::<Vec<_>>()
                .tap_mut(|e| {
                    e.insert(0, None);
                });

                let sql =
                    format!("SELECT {encrypted_col} FROM encrypted ORDER BY {encrypted_col} DESC NULLS FIRST");

                let result = query::<Option<$type>>(&sql).await;
                assert_expected(&expected, &result);

                let result = simple_query_with_null(&sql).await;
                assert_expected_as_string(&expected, &result);

            }
        };
    }

    // ------------------------------------------------------------------------
    // DESC NULLS LAST
    // [Some("E"), Some("D"), Some("C"), Some("B"), Some("A"), None]
    macro_rules! test_order_by_with_null_desc_nulls_last {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();
                clear().await;
                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));

                // insert a null value (before the test values to avoid any natural ordering issues)
                let encrypted_value: Option<$type> = None;
                insert_encrypted_value(&encrypted_col, &encrypted_value).await;

                // insert test values
                let expected = insert_encrypted_values!($type, &encrypted_col);

                // NULL values are Option<T> in tokio_postgres
                // Wrap expected values in Some
                // [Some("E"), Some("D"), Some("C"), Some("B"), Some("A"), None]
                let expected = expected.tap_mut(|e| e.reverse()).into_iter().map(|v| Some(v)).collect::<Vec<_>>()
                .tap_mut(|e| {
                    e.push(None);
                });

                let sql =
                    format!("SELECT {encrypted_col} FROM encrypted ORDER BY {encrypted_col} DESC NULLS LAST");


                let result = query::<Option<$type>>(&sql).await;
                assert_expected(&expected, &result);

                let result = simple_query_with_null(&sql).await;
                assert_expected_as_string(&expected, &result);

            }
        };
    }

    // ============================================================

    test_order_by_with_null_asc!(order_by_int2_with_null_asc, i16, int2);
    test_order_by_with_null_asc!(order_by_int4_with_null_asc, i32, int4);
    test_order_by_with_null_asc!(order_by_int8_with_null_asc, i64, int8);
    test_order_by_with_null_asc!(order_by_floatwith_null_asc, f64, float8);
    test_order_by_with_null_asc!(order_by_text_with_null_asc, String, text);
    test_order_by_with_null_asc!(order_by_date_with_null_asc, NaiveDate, date);

    test_order_by_with_null_asc_nulls_last!(order_by_int2_with_null_asc_nulls_last, i16, int2);
    test_order_by_with_null_asc_nulls_last!(order_by_int4_with_null_asc_nulls_last, i32, int4);
    test_order_by_with_null_asc_nulls_last!(order_by_int8_with_null_asc_nulls_last, i64, int8);
    test_order_by_with_null_asc_nulls_last!(order_by_floatwith_null_asc_nulls_last, f64, float8);
    test_order_by_with_null_asc_nulls_last!(order_by_text_with_null_asc_nulls_last, String, text);
    test_order_by_with_null_asc_nulls_last!(
        order_by_date_with_null_asc_nulls_last,
        NaiveDate,
        date
    );

    test_order_by_with_null_asc_nulls_first!(order_by_int2_with_null_asc_nulls_first, i16, int2);
    test_order_by_with_null_asc_nulls_first!(order_by_int4_with_null_asc_nulls_first, i32, int4);
    test_order_by_with_null_asc_nulls_first!(order_by_int8_with_null_asc_nulls_first, i64, int8);
    test_order_by_with_null_asc_nulls_first!(order_by_floatwith_null_asc_nulls_first, f64, float8);
    test_order_by_with_null_asc_nulls_first!(order_by_text_with_null_asc_nulls_first, String, text);
    test_order_by_with_null_asc_nulls_first!(
        order_by_date_with_null_asc_nulls_first,
        NaiveDate,
        date
    );

    test_order_by_with_null_desc!(order_by_int2_with_null_desc, i16, int2);
    test_order_by_with_null_desc!(order_by_int4_with_null_desc, i32, int4);
    test_order_by_with_null_desc!(order_by_int8_with_null_desc, i64, int8);
    test_order_by_with_null_desc!(order_by_floatwith_null_desc, f64, float8);
    test_order_by_with_null_desc!(order_by_text_with_null_desc, String, text);
    test_order_by_with_null_desc!(order_by_date_with_null_desc, NaiveDate, date);

    test_order_by_with_null_desc_nulls_first!(order_by_int2_with_null_desc_nulls_first, i16, int2);
    test_order_by_with_null_desc_nulls_first!(order_by_int4_with_null_desc_nulls_first, i32, int4);
    test_order_by_with_null_desc_nulls_first!(order_by_int8_with_null_desc_nulls_first, i64, int8);
    test_order_by_with_null_desc_nulls_first!(
        order_by_floatwith_null_desc_nulls_first,
        f64,
        float8
    );
    test_order_by_with_null_desc_nulls_first!(
        order_by_text_with_null_desc_nulls_first,
        String,
        text
    );
    test_order_by_with_null_desc_nulls_first!(
        order_by_date_with_null_desc_nulls_first,
        NaiveDate,
        date
    );

    test_order_by_with_null_desc_nulls_last!(order_by_int2_with_null_desc_nulls_last, i16, int2);
    test_order_by_with_null_desc_nulls_last!(order_by_int4_with_null_desc_nulls_last, i32, int4);
    test_order_by_with_null_desc_nulls_last!(order_by_int8_with_null_desc_nulls_last, i64, int8);
    test_order_by_with_null_desc_nulls_last!(order_by_floatwith_null_desc_nulls_last, f64, float8);
    test_order_by_with_null_desc_nulls_last!(order_by_text_with_null_desc_nulls_last, String, text);
    test_order_by_with_null_desc_nulls_last!(
        order_by_date_with_null_desc_nulls_last,
        NaiveDate,
        date
    );
}
