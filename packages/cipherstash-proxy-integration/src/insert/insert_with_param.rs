#[cfg(test)]
mod tests {
    use crate::common::{clear, execute_query, query_by, random_id, random_limited, trace};
    use chrono::NaiveDate;
    use serde_json::Value;

    macro_rules! test_insert_with_param {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;

                let id = random_id();

                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));
                let encrypted_val = crate::value_for_type!($type, random_limited());

                let sql = format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ($1, $2)");
                execute_query(&sql, &[&id, &encrypted_val]).await;

                let expected = vec![encrypted_val];

                let sql = format!("SELECT {encrypted_col} FROM encrypted WHERE id = $1");

                let actual = query_by::<$type>(&sql, &id).await;

                assert_eq!(expected, actual);
            }
        };
    }

    test_insert_with_param!(insert_with_param_int2, i16, int2);
    test_insert_with_param!(insert_with_param_int4, i32, int4);
    test_insert_with_param!(insert_with_param_int8, i64, int8);
    test_insert_with_param!(insert_with_param_float8, f64, float8);
    test_insert_with_param!(insert_with_param_bool, bool, bool);
    test_insert_with_param!(insert_with_param_text, String, text);
    test_insert_with_param!(insert_with_param_date, NaiveDate, date);
    test_insert_with_param!(insert_with_param_jsonb, Value, jsonb);

    // -----------------------------------------------------------------

    /// Sanity check insert of unencrypted plaintext value
    #[tokio::test]
    pub async fn insert_with_param_plaintext() {
        trace();

        clear().await;

        let id = random_id();

        let encrypted_val = crate::value_for_type!(String, random_limited());

        let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
        execute_query(sql, &[&id, &encrypted_val]).await;

        let expected = vec![encrypted_val];

        let sql = "SELECT plaintext FROM encrypted WHERE id = $1";

        let actual = query_by::<String>(sql, &id).await;

        assert_eq!(expected, actual);
    }
}
