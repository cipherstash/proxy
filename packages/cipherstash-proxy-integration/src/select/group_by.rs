#[cfg(test)]
mod tests {
    use crate::common::{clear, insert, query, random_id, simple_query, trace};
    use chrono::NaiveDate;

    macro_rules! value_for_type {
        (String, $i:expr) => {
            format!("group_{}", $i)
        };

        (NaiveDate, $i:expr) => {
            NaiveDate::parse_from_str(&format!("2023-01-{}", $i), "%Y-%m-%d").unwrap()
        };

        ($type:ident, $i:expr) => {
            $i as $type
        };
    }

    macro_rules! test_group_by {
        ($name: ident, $type: ident, $pg_type: ident) => {
            #[tokio::test]
            pub async fn $name() {
                trace();

                clear().await;

                let encrypted_col = format!("encrypted_{}", stringify!($pg_type));

                for i in 1..=10 {
                    let encrypted_val = value_for_type!($type, i);

                    // Create two records with the same encrypted_int4 value
                    for _ in 1..=2 {
                        let id = random_id();
                        let sql =
                            format!("INSERT INTO encrypted (id, {encrypted_col}) VALUES ($1, $2)");
                        insert(&sql, &[&id, &encrypted_val]).await;
                    }
                }

                // Validate that there are 20 records in the encrypted table
                let sql = format!("SELECT {encrypted_col} FROM encrypted");

                let rows = query::<$type>(&sql).await;
                assert_eq!(rows.len(), 20);

                let rows = simple_query::<$type>(&sql).await;
                assert_eq!(rows.len(), 20);

                // GROUP BY should return 10 records, each representing two records with the same encrypted_int4 value
                let sql = format!("SELECT COUNT(*) FROM encrypted GROUP BY {encrypted_col}");

                let rows = query::<i64>(&sql).await;
                assert_eq!(rows.len(), 10);

                let rows = simple_query::<i64>(&sql).await;
                assert_eq!(rows.len(), 10);
            }
        };
    }

    test_group_by!(group_by_int2, i16, int2);
    test_group_by!(group_by_int4, i32, int4);
    test_group_by!(group_by_int8, i64, int8);
    test_group_by!(group_by_float8, f64, float8);
    test_group_by!(group_by_text, String, text);
    test_group_by!(group_by_date, NaiveDate, date);
}
