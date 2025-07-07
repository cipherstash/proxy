#[cfg(test)]
mod tests {
    use crate::common::{clear, execute_query, query_by, random_id, trace};
    use chrono::NaiveDate;
    use serde_json::Value;

    macro_rules! test_update_with_null_param {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;

                let id = random_id();

                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));
                let initial_val = crate::value_for_type!($type, 1);
                let encrypted_val: Option<$type> = None;

                // First insert a record with a value
                let sql = format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ($1, $2)");
                execute_query(&sql, &[&id, &initial_val]).await;

                // Then update it to NULL
                let sql = format!("UPDATE encrypted SET {encrypted_col} = $1 WHERE id = $2");
                execute_query(&sql, &[&encrypted_val, &id]).await;

                let expected = vec![encrypted_val];

                let sql = format!("SELECT {encrypted_col} FROM encrypted WHERE id = $1");
                let result = query_by::<Option<$type>>(&sql, &id).await;

                assert_eq!(expected, result);
            }
        };
    }

    test_update_with_null_param!(update_with_null_param_int2, i16, int2);
    test_update_with_null_param!(update_with_null_param_int4, i32, int4);
    test_update_with_null_param!(update_with_null_param_int8, i64, int8);
    test_update_with_null_param!(update_with_null_param_float8, f64, float8);
    test_update_with_null_param!(update_with_null_param_bool, bool, bool);
    test_update_with_null_param!(update_with_null_param_text_only, String, text);
    test_update_with_null_param!(update_with_null_param_date, NaiveDate, date);
    test_update_with_null_param!(update_with_null_param_jsonb, Value, jsonb);
}
