#[cfg(test)]
mod tests {
    use crate::common::{
        clear, execute_query, execute_simple_query, query_by, random_id, random_limited,
        simple_query_with_null, trace,
    };
    use chrono::NaiveDate;
    use serde_json::Value;

    macro_rules! test_update_with_null_literal {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;

                let id = random_id();

                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));
                let initial_val = crate::value_for_type!($type, random_limited());
                let encrypted_val: Option<$type> = None;

                // First insert a record with a value
                let sql = format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ($1, $2)");
                execute_query(&sql, &[&id, &initial_val]).await;

                // Then update it to NULL
                let sql = format!("UPDATE encrypted SET {encrypted_col} = NULL WHERE id = $1");
                execute_query(&sql, &[&id]).await;

                let expected = vec![encrypted_val];

                let sql = format!("SELECT {encrypted_col} FROM encrypted WHERE id = $1");

                let actual = query_by::<Option<$type>>(&sql, &id).await;

                assert_eq!(expected, actual);
            }
        };
    }

    test_update_with_null_literal!(update_with_null_literal_int2, i16, int2);
    test_update_with_null_literal!(update_with_null_literal_int4, i32, int4);
    test_update_with_null_literal!(update_with_null_literal_int8, i64, int8);
    test_update_with_null_literal!(update_with_null_literal_float8, f64, float8);
    test_update_with_null_literal!(update_with_null_literal_bool, bool, bool);
    test_update_with_null_literal!(update_with_null_literal_text, String, text);
    test_update_with_null_literal!(update_with_null_literal_date, NaiveDate, date);
    test_update_with_null_literal!(update_with_null_literal_jsonb, Value, jsonb);

    macro_rules! test_update_simple_query_with_null_literal {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;

                let id = random_id();

                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));
                let initial_val = crate::value_for_type!($type, random_limited());
                let encrypted_val: Option<String> = None;

                let insert_sql =
                    format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ({id}, '{initial_val}')");
                let update_sql = format!("UPDATE encrypted SET {encrypted_col} = NULL WHERE id = {id}");
                let select_sql = format!("SELECT {encrypted_col} FROM encrypted WHERE id = {id}");

                let expected = vec![encrypted_val];

                execute_simple_query(&insert_sql).await;
                execute_simple_query(&update_sql).await;
                let actual = simple_query_with_null(&select_sql).await;

                assert_eq!(expected, actual);
            }
        };
    }

    test_update_simple_query_with_null_literal!(
        update_simple_query_with_null_literal_int2,
        i16,
        int2
    );
    test_update_simple_query_with_null_literal!(
        update_simple_query_with_null_literal_int4,
        i32,
        int4
    );
    test_update_simple_query_with_null_literal!(
        update_simple_query_with_null_literal_int8,
        i64,
        int8
    );
    test_update_simple_query_with_null_literal!(
        update_simple_query_with_null_literal_float8,
        f64,
        float8
    );
    test_update_simple_query_with_null_literal!(
        update_simple_query_with_null_literal_bool,
        bool,
        bool
    );
    test_update_simple_query_with_null_literal!(
        update_simple_query_with_null_literal_text,
        String,
        text
    );
    test_update_simple_query_with_null_literal!(
        update_simple_query_with_null_literal_date,
        NaiveDate,
        date
    );
    test_update_simple_query_with_null_literal!(
        update_simple_query_with_null_literal_jsonb,
        Value,
        jsonb
    );

    // -----------------------------------------------------------------

    /// Sanity check update of unencrypted literal value to NULL
    #[tokio::test]
    pub async fn update_with_null_literal_plaintext() {
        trace();

        clear().await;

        let id = random_id();

        let initial_val = "initial_value";
        let expected: Vec<Option<String>> = vec![None];

        // First insert a record
        let sql = format!("INSERT INTO encrypted (id, plaintext) VALUES ($1, '{initial_val}')");
        execute_query(&sql, &[&id]).await;

        // Then update it to NULL
        let sql = "UPDATE encrypted SET plaintext = NULL WHERE id = $1";
        execute_query(sql, &[&id]).await;

        let sql = "SELECT plaintext FROM encrypted WHERE id = $1";

        let actual = query_by::<Option<String>>(sql, &id).await;

        assert_eq!(expected, actual);
    }
}
