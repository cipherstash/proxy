#[cfg(test)]
mod tests {
    use crate::common::{clear, execute_query, query_by, random_id, random_limited, trace};
    use chrono::NaiveDate;
    use serde_json::Value;

    macro_rules! test_update_with_param {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;

                let id = random_id();

                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));
                let initial_val = crate::value_for_type!($type, 1);
                let encrypted_val = crate::value_for_type!($type, random_limited());

                // First insert a record with initial value
                let sql = format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ($1, $2)");
                execute_query(&sql, &[&id, &initial_val]).await;

                // Then update it with new value
                let sql = format!("UPDATE encrypted SET {encrypted_col} = $1 WHERE id = $2");
                execute_query(&sql, &[&encrypted_val, &id]).await;

                let expected = vec![encrypted_val];

                let sql = format!("SELECT {encrypted_col} FROM encrypted WHERE id = $1");

                let actual = query_by::<$type>(&sql, &id).await;

                assert_eq!(expected, actual);
            }
        };
    }

    test_update_with_param!(update_with_param_int2, i16, int2);
    test_update_with_param!(update_with_param_int4, i32, int4);
    test_update_with_param!(update_with_param_int8, i64, int8);
    test_update_with_param!(update_with_param_float8, f64, float8);
    test_update_with_param!(update_with_param_bool, bool, bool);
    test_update_with_param!(update_with_param_text, String, text);
    test_update_with_param!(update_with_param_date, NaiveDate, date);
    test_update_with_param!(update_with_param_jsonb, Value, jsonb);

    // -----------------------------------------------------------------

    /// Sanity check update of unencrypted plaintext value
    #[tokio::test]
    pub async fn update_with_param_plaintext() {
        trace();

        clear().await;

        let id = random_id();

        let initial_val = crate::value_for_type!(String, 1);
        let encrypted_val = crate::value_for_type!(String, random_limited());

        // First insert a record
        let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
        execute_query(sql, &[&id, &initial_val]).await;

        // Then update it
        let sql = "UPDATE encrypted SET plaintext = $1 WHERE id = $2";
        execute_query(sql, &[&encrypted_val, &id]).await;

        let expected = vec![encrypted_val];

        let sql = "SELECT plaintext FROM encrypted WHERE id = $1";

        let actual = query_by::<String>(sql, &id).await;

        assert_eq!(expected, actual);
    }
}
