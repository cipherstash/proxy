#[cfg(test)]
mod tests {
    use crate::common::{
        clear_ope, connect_with_tls, interleaved_indices, random_id, trace, PROXY,
    };

    #[tokio::test]
    async fn map_ope_order_text_asc() {
        trace();
        clear_ope().await;
        let client = connect_with_tls(PROXY).await;

        let values = ["aardvark", "aplomb", "chimera", "chrysalis", "zephyr"];

        let insert = "INSERT INTO encrypted_ope (id, encrypted_text) VALUES ($1, $2)";
        for idx in interleaved_indices(values.len()) {
            client
                .query(insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let rows = client
            .query(
                "SELECT encrypted_text FROM encrypted_ope ORDER BY encrypted_text",
                &[],
            )
            .await
            .unwrap();

        let actual: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
        let expected: Vec<String> = values.iter().map(|s| s.to_string()).collect();

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ope_order_text_desc() {
        trace();
        clear_ope().await;
        let client = connect_with_tls(PROXY).await;

        let values = ["aardvark", "aplomb", "chimera", "chrysalis", "zephyr"];

        let insert = "INSERT INTO encrypted_ope (id, encrypted_text) VALUES ($1, $2)";
        for idx in interleaved_indices(values.len()) {
            client
                .query(insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let rows = client
            .query(
                "SELECT encrypted_text FROM encrypted_ope ORDER BY encrypted_text DESC",
                &[],
            )
            .await
            .unwrap();

        let actual: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
        let expected: Vec<String> = values.iter().rev().map(|s| s.to_string()).collect();

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ope_order_int4_asc() {
        trace();
        clear_ope().await;
        let client = connect_with_tls(PROXY).await;

        let values: Vec<i32> = vec![-100, -1, 0, 1, 42, 1000, i32::MAX];

        let insert = "INSERT INTO encrypted_ope (id, encrypted_int4) VALUES ($1, $2)";
        for idx in interleaved_indices(values.len()) {
            client
                .query(insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let rows = client
            .query(
                "SELECT encrypted_int4 FROM encrypted_ope ORDER BY encrypted_int4",
                &[],
            )
            .await
            .unwrap();

        let actual: Vec<i32> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(actual, values);
    }

    #[tokio::test]
    async fn map_ope_order_int4_desc() {
        trace();
        clear_ope().await;
        let client = connect_with_tls(PROXY).await;

        let values: Vec<i32> = vec![-100, -1, 0, 1, 42, 1000, i32::MAX];

        let insert = "INSERT INTO encrypted_ope (id, encrypted_int4) VALUES ($1, $2)";
        for idx in interleaved_indices(values.len()) {
            client
                .query(insert, &[&random_id(), &values[idx]])
                .await
                .unwrap();
        }

        let rows = client
            .query(
                "SELECT encrypted_int4 FROM encrypted_ope ORDER BY encrypted_int4 DESC",
                &[],
            )
            .await
            .unwrap();

        let actual: Vec<i32> = rows.iter().map(|r| r.get(0)).collect();
        let expected: Vec<i32> = values.into_iter().rev().collect();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn map_ope_order_nulls_last_by_default() {
        trace();
        clear_ope().await;
        let client = connect_with_tls(PROXY).await;

        client
            .query(
                "INSERT INTO encrypted_ope (id) VALUES ($1)",
                &[&random_id()],
            )
            .await
            .unwrap();

        client
            .query(
                "INSERT INTO encrypted_ope (id, encrypted_text) VALUES ($1, $2), ($3, $4)",
                &[&random_id(), &"a", &random_id(), &"b"],
            )
            .await
            .unwrap();

        let rows = client
            .query(
                "SELECT encrypted_text FROM encrypted_ope ORDER BY encrypted_text",
                &[],
            )
            .await
            .unwrap();

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
        clear_ope().await;
        let client = connect_with_tls(PROXY).await;

        client
            .query(
                "INSERT INTO encrypted_ope (id, encrypted_text) VALUES ($1, $2), ($3, $4)",
                &[&random_id(), &"a", &random_id(), &"b"],
            )
            .await
            .unwrap();

        client
            .query(
                "INSERT INTO encrypted_ope (id) VALUES ($1)",
                &[&random_id()],
            )
            .await
            .unwrap();

        let rows = client
            .query(
                "SELECT encrypted_text FROM encrypted_ope ORDER BY encrypted_text NULLS FIRST",
                &[],
            )
            .await
            .unwrap();

        let actual: Vec<Option<String>> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(
            actual,
            vec![None, Some("a".into()), Some("b".into())],
            "NULLS FIRST should explicitly sort NULLs first"
        );
    }
}
