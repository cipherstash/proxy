#[cfg(test)]
mod tests {
    use crate::common::{
        clear, execute_query, execute_simple_query, query_by, random_id, simple_query_with_null,
        trace,
    };
    use chrono::NaiveDate;
    use serde_json::Value;

    macro_rules! test_insert_with_null_literal {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;

                let id = random_id();

                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));
                let encrypted_val: Option<$type> = None;

                let sql = format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ($1, NULL)");
                execute_query(&sql, &[&id]).await;

                let expected = vec![encrypted_val];

                let sql = format!("SELECT {encrypted_col} FROM encrypted WHERE id = $1");

                let actual = query_by::<Option<$type>>(&sql, &id).await;

                assert_eq!(expected, actual);
            }
        };
    }

    test_insert_with_null_literal!(insert_with_null_literal_int2, i16, int2);
    test_insert_with_null_literal!(insert_with_null_literal_int4, i32, int4);
    test_insert_with_null_literal!(insert_with_null_literal_int8, i64, int8);
    test_insert_with_null_literal!(insert_with_null_literal_float8, f64, float8);
    test_insert_with_null_literal!(insert_with_null_literal_bool, bool, bool);
    test_insert_with_null_literal!(insert_with_null_literal_text, String, text);
    test_insert_with_null_literal!(insert_with_null_literal_date, NaiveDate, date);
    test_insert_with_null_literal!(insert_with_null_literal_jsonb, Value, jsonb);

    macro_rules! test_insert_simple_query_with_null_literal {
        ($name: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;

                let id = random_id();

                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));
                let encrypted_val: Option<String> = None;

                let insert_sql =
                    format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ({id}, NULL)");
                let select_sql = format!("SELECT {encrypted_col} FROM encrypted WHERE id = {id}");

                let expected = vec![encrypted_val];

                execute_simple_query(&insert_sql).await;
                let actual = simple_query_with_null(&select_sql).await;

                assert_eq!(expected, actual);
            }
        };
    }

    test_insert_simple_query_with_null_literal!(insert_simple_query_with_null_literal_int2, int2);
    test_insert_simple_query_with_null_literal!(insert_simple_query_with_null_literal_int4, int4);
    test_insert_simple_query_with_null_literal!(insert_simple_query_with_null_literal_int8, int8);
    test_insert_simple_query_with_null_literal!(
        insert_simple_query_with_null_literal_float8,
        float8
    );
    test_insert_simple_query_with_null_literal!(insert_simple_query_with_null_literal_bool, bool);
    test_insert_simple_query_with_null_literal!(insert_simple_query_with_null_literal_text, text);
    test_insert_simple_query_with_null_literal!(insert_simple_query_with_null_literal_date, date);
    test_insert_simple_query_with_null_literal!(insert_simple_query_with_null_literal_jsonb, jsonb);

    // -----------------------------------------------------------------

    /// Sanity check insert of unencrypted literal value
    #[tokio::test]
    pub async fn insert_with_null_literal_plaintext() {
        trace();

        clear().await;

        let id = random_id();

        let expected: Vec<Option<String>> = vec![None];

        let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, NULL)";
        execute_query(sql, &[&id]).await;

        let sql = "SELECT plaintext FROM encrypted WHERE id = $1";

        let actual = query_by::<Option<String>>(sql, &id).await;

        assert_eq!(expected, actual);
    }
}
