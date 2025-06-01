#[cfg(test)]
mod tests {
    use crate::common::{clear, insert, query, random_id, random_limited, trace};
    use chrono::NaiveDate;
    use rand::{seq::IndexedRandom, Rng};
    use serde_json::Value;
    use tokio_postgres::types::ToSql;
    use tracing::info;

    fn value_for_type(t: &str) -> Box<dyn ToSql + Sync> {
        let mut rng = rand::rng();

        match t {
            "i16" => Box::new(rng.random_range(1..=i16::MAX) as i16),
            "i32" => Box::new(rng.random_range(1..=i32::MAX) as i32),
            "i64" => Box::new(rng.random_range(1..=i64::MAX) as i64),
            "f64" => Box::new(rng.random_range(1.0..=f64::MAX) as f64),
            "bool" => Box::new(rand::random_bool(0.5) as bool),
            "String" => {
                let i = random_limited();
                Box::new(((b'A' + (i - 1) as u8) as char).to_string())
            }
            "NaiveDate" => {
                let i = random_limited();
                Box::new(NaiveDate::parse_from_str(&format!("2023-01-{}", i), "%Y-%m-%d").unwrap())
            }
            "Value" => {
                let i = rng.random_range(1..=i32::MAX) as i32;
                Box::new(serde_json::json!({"n": i, "s": format!("{}", i) }))
            }

            _ => panic!("Unknown type {t}"),
        }
    }

    ///
    /// Generates a random number of columns and values
    /// Return as a tuple of two vecs:
    ///     - first vec contains column names
    ///     - second vec contains values of the corresponding column type
    pub fn generate_columns_with_values() -> (Vec<String>, Vec<Box<(dyn ToSql + Sync)>>) {
        let columns = vec![
            ("i16", "int2"),
            ("i32", "int4"),
            ("i64", "int8"),
            ("f64", "float8"),
            ("bool", "bool"),
            ("String", "text"),
            ("NaiveDate", "date"),
            ("Value", "jsonb"),
        ];

        let mut rng = rand::rng();
        let n = rng.random_range(1..columns.len());

        let (mut columns, mut values): (Vec<_>, Vec<_>) = columns
            .choose_multiple(&mut rng, n)
            .map(|(t, c)| {
                let c = format!("encrypted_{c}");
                (c, value_for_type(t))
            })
            .unzip();

        let id = Box::new(random_id());
        columns.insert(0, "id".to_string());
        values.insert(0, id);

        (columns, values)
    }

    pub async fn query<T: for<'a> tokio_postgres::types::FromSql<'a> + Send + Sync>(
        sql: &str,
    ) -> Vec<T> {
        let client = connect_with_tls(PROXY).await;
        let rows = client.query(sql, &[]).await.unwrap();
        rows.iter().map(|row| row.get(0)).collect::<Vec<T>>()
    }

    #[tokio::test]
    pub async fn test_everything_all_at_once() {
        trace();

        clear().await;

        let (columns, values) = generate_columns_with_values();

        info!("Columns: {:?}", columns.join(","));
        info!("Values: {:?}", values);

        let columns = columns.join(", ");
        let params: Vec<&(dyn ToSql + Sync)> = values.iter().map(|v| v.as_ref()).collect();

        let placeholders = (1..=values.len())
            .map(|i| format!("${}", i))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!("INSERT INTO encrypted ({columns}) VALUES ({placeholders})");

        info!(sql);
        insert(&sql, &params).await;

        let sql = format!("SELECT {columns} FROM encrypted WHERE id = $1");

        // let actual = query_by::<$type>(&sql, &id).await;

        // assert_eq!(expected, actual);
    }

    // test_insert_with_params!(insert_with_params_int2, i16, int2);
    // test_insert_with_params!(insert_with_params_int4, i32, int4);
    // test_insert_with_params!(insert_with_params_int8, i64, int8);
    // test_insert_with_params!(insert_with_params_float8, f64, float8);
    // test_insert_with_params!(insert_with_params_bool, bool, bool);
    // test_insert_with_params!(insert_with_params_text_only, String, text);
    // test_insert_with_params!(insert_with_params_date, NaiveDate, date);
    // test_insert_with_params!(insert_with_params_jsonb, Value, jsonb);
}
