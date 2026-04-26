#[cfg(test)]
mod tests {
    use crate::common::{
        clear_table, connect_with_tls, interleaved_indices, random_id, trace, PROXY,
    };

    #[tokio::test]
    async fn map_ope_order_text_asc() {
        trace();
        let table = "encrypted_ope_order_text_asc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let values = ["aardvark", "aplomb", "chimera", "chrysalis", "zephyr"];

        let insert = format!("INSERT INTO {table} (id, encrypted_text) VALUES ($1, $2)");
        for idx in interleaved_indices(values.len()) {
            client
                .query(&insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let select = format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text");
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
        let expected: Vec<String> = values.iter().map(|s| s.to_string()).collect();

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ope_order_text_desc() {
        trace();
        let table = "encrypted_ope_order_text_desc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let values = ["aardvark", "aplomb", "chimera", "chrysalis", "zephyr"];

        let insert = format!("INSERT INTO {table} (id, encrypted_text) VALUES ($1, $2)");
        for idx in interleaved_indices(values.len()) {
            client
                .query(&insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let select = format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text DESC");
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
        let expected: Vec<String> = values.iter().rev().map(|s| s.to_string()).collect();

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ope_order_int4_asc() {
        trace();
        let table = "encrypted_ope_order_int4_asc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let values: Vec<i32> = vec![-100, -1, 0, 1, 42, 1000, i32::MAX];

        let insert = format!("INSERT INTO {table} (id, encrypted_int4) VALUES ($1, $2)");
        for idx in interleaved_indices(values.len()) {
            client
                .query(&insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let select = format!("SELECT encrypted_int4 FROM {table} ORDER BY encrypted_int4");
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<i32> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(actual, values);
    }

    #[tokio::test]
    async fn map_ope_order_int4_desc() {
        trace();
        let table = "encrypted_ope_order_int4_desc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let values: Vec<i32> = vec![-100, -1, 0, 1, 42, 1000, i32::MAX];

        let insert = format!("INSERT INTO {table} (id, encrypted_int4) VALUES ($1, $2)");
        for idx in interleaved_indices(values.len()) {
            client
                .query(&insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let select = format!("SELECT encrypted_int4 FROM {table} ORDER BY encrypted_int4 DESC");
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<i32> = rows.iter().map(|r| r.get(0)).collect();
        let expected: Vec<i32> = values.into_iter().rev().collect();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ope_order_nulls_last_by_default() {
        trace();
        let table = "encrypted_ope_order_nulls_last";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let null_insert = format!("INSERT INTO {table} (id) VALUES ($1)");
        client.query(&null_insert, &[&random_id()]).await.unwrap();

        let insert =
            format!("INSERT INTO {table} (id, encrypted_text) VALUES ($1, $2), ($3, $4)");
        client
            .query(&insert, &[&random_id(), &"a", &random_id(), &"b"])
            .await
            .unwrap();

        let select = format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text");
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<Option<String>> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(
            actual,
            vec![Some("a".into()), Some("b".into()), None],
            "NULLs should sort last by default"
        );
    }

    #[tokio::test]
    async fn map_ope_order_nulls_first() {
        trace();
        let table = "encrypted_ope_order_nulls_first";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;

        let insert =
            format!("INSERT INTO {table} (id, encrypted_text) VALUES ($1, $2), ($3, $4)");
        client
            .query(&insert, &[&random_id(), &"a", &random_id(), &"b"])
            .await
            .unwrap();

        let null_insert = format!("INSERT INTO {table} (id) VALUES ($1)");
        client.query(&null_insert, &[&random_id()]).await.unwrap();

        let select = format!(
            "SELECT encrypted_text FROM {table} ORDER BY encrypted_text NULLS FIRST"
        );
        let rows = client.query(&select, &[]).await.unwrap();

        let actual: Vec<Option<String>> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(
            actual,
            vec![None, Some("a".into()), Some("b".into())],
            "NULLS FIRST should explicitly sort NULLs first"
        );
    }
}
