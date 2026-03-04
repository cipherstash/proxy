#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, trace, PROXY};
    use crate::ore_order_helpers;
    use crate::ore_order_helpers::SortDirection;
    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn map_ore_order_text() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_text(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_text_desc() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_text_desc(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_nulls_last_by_default() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_nulls_last_by_default(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_nulls_first() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_nulls_first(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_qualified_column() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_qualified_column(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_qualified_column_with_alias() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_qualified_column_with_alias(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_no_eql_column_in_select_projection() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_no_eql_column_in_select_projection(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn can_order_by_plaintext_column() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_plaintext_column(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn can_order_by_plaintext_and_eql_columns() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_plaintext_and_eql_columns(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_simple_protocol() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_simple_protocol(&client).await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_int2() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i16> = vec![-100, -10, -1, 0, 1, 5, 10, 20, 100, 200];
        ore_order_helpers::ore_order_generic(&client, "encrypted_int2", values, SortDirection::Asc)
            .await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_int2_desc() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i16> = vec![-100, -10, -1, 0, 1, 5, 10, 20, 100, 200];
        ore_order_helpers::ore_order_generic(
            &client,
            "encrypted_int2",
            values,
            SortDirection::Desc,
        )
        .await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_int4() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i32> = vec![
            -50_000, -1_000, -1, 0, 1, 42, 1_000, 10_000, 50_000, 100_000,
        ];
        ore_order_helpers::ore_order_generic(&client, "encrypted_int4", values, SortDirection::Asc)
            .await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_int4_desc() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i32> = vec![
            -50_000, -1_000, -1, 0, 1, 42, 1_000, 10_000, 50_000, 100_000,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            "encrypted_int4",
            values,
            SortDirection::Desc,
        )
        .await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_int8() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i64> = vec![
            -1_000_000, -10_000, -1, 0, 1, 42, 10_000, 100_000, 1_000_000, 9_999_999,
        ];
        ore_order_helpers::ore_order_generic(&client, "encrypted_int8", values, SortDirection::Asc)
            .await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_int8_desc() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i64> = vec![
            -1_000_000, -10_000, -1, 0, 1, 42, 10_000, 100_000, 1_000_000, 9_999_999,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            "encrypted_int8",
            values,
            SortDirection::Desc,
        )
        .await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_float8() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<f64> = vec![
            -99.9, -1.5, -0.001, 0.0, 0.001, 1.5, 3.25, 42.0, 99.9, 1000.5,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            "encrypted_float8",
            values,
            SortDirection::Asc,
        )
        .await;
    }

    #[tokio::test]
    #[serial]
    async fn map_ore_order_float8_desc() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<f64> = vec![
            -99.9, -1.5, -0.001, 0.0, 0.001, 1.5, 3.25, 42.0, 99.9, 1000.5,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            "encrypted_float8",
            values,
            SortDirection::Desc,
        )
        .await;
    }
}
