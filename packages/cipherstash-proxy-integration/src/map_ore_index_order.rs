#[cfg(test)]
mod tests {
    use crate::common::{clear_table, connect_with_tls, trace, PROXY};
    use crate::ore_order_helpers;
    use crate::ore_order_helpers::SortDirection;

    #[tokio::test]
    async fn map_ore_order_text() {
        trace();
        let table = "encrypted_ore_order_text";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_text(&client, table).await;
    }

    #[tokio::test]
    async fn map_ore_order_text_desc() {
        trace();
        let table = "encrypted_ore_order_text_desc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_text_desc(&client, table).await;
    }

    #[tokio::test]
    async fn map_ore_order_nulls_last_by_default() {
        trace();
        let table = "encrypted_ore_order_nulls_last";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_nulls_last_by_default(&client, table).await;
    }

    #[tokio::test]
    async fn map_ore_order_nulls_first() {
        trace();
        let table = "encrypted_ore_order_nulls_first";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_nulls_first(&client, table).await;
    }

    #[tokio::test]
    async fn map_ore_order_qualified_column() {
        trace();
        let table = "encrypted_ore_order_qualified";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_qualified_column(&client, table).await;
    }

    #[tokio::test]
    async fn map_ore_order_qualified_column_with_alias() {
        trace();
        let table = "encrypted_ore_order_qualified_alias";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_qualified_column_with_alias(&client, table).await;
    }

    #[tokio::test]
    async fn map_ore_order_no_eql_column_in_select_projection() {
        trace();
        let table = "encrypted_ore_order_no_select_projection";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_no_eql_column_in_select_projection(&client, table).await;
    }

    #[tokio::test]
    async fn can_order_by_plaintext_column() {
        trace();
        let table = "encrypted_ore_order_plaintext_column";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_plaintext_column(&client, table).await;
    }

    #[tokio::test]
    async fn can_order_by_plaintext_and_eql_columns() {
        trace();
        let table = "encrypted_ore_order_plaintext_and_eql";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_plaintext_and_eql_columns(&client, table).await;
    }

    #[tokio::test]
    async fn map_ore_order_simple_protocol() {
        trace();
        let table = "encrypted_ore_order_simple_protocol";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        ore_order_helpers::ore_order_simple_protocol(&client, table).await;
    }

    #[tokio::test]
    async fn map_ore_order_int2() {
        trace();
        let table = "encrypted_ore_order_int2";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i16> = vec![-100, -10, -1, 0, 1, 5, 10, 20, 100, 200];
        ore_order_helpers::ore_order_generic(
            &client,
            table,
            "encrypted_int2",
            values,
            SortDirection::Asc,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ore_order_int2_desc() {
        trace();
        let table = "encrypted_ore_order_int2_desc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i16> = vec![-100, -10, -1, 0, 1, 5, 10, 20, 100, 200];
        ore_order_helpers::ore_order_generic(
            &client,
            table,
            "encrypted_int2",
            values,
            SortDirection::Desc,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ore_order_int4() {
        trace();
        let table = "encrypted_ore_order_int4";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i32> = vec![
            -50_000, -1_000, -1, 0, 1, 42, 1_000, 10_000, 50_000, 100_000,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            table,
            "encrypted_int4",
            values,
            SortDirection::Asc,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ore_order_int4_desc() {
        trace();
        let table = "encrypted_ore_order_int4_desc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i32> = vec![
            -50_000, -1_000, -1, 0, 1, 42, 1_000, 10_000, 50_000, 100_000,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            table,
            "encrypted_int4",
            values,
            SortDirection::Desc,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ore_order_int8() {
        trace();
        let table = "encrypted_ore_order_int8";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i64> = vec![
            -1_000_000, -10_000, -1, 0, 1, 42, 10_000, 100_000, 1_000_000, 9_999_999,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            table,
            "encrypted_int8",
            values,
            SortDirection::Asc,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ore_order_int8_desc() {
        trace();
        let table = "encrypted_ore_order_int8_desc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<i64> = vec![
            -1_000_000, -10_000, -1, 0, 1, 42, 10_000, 100_000, 1_000_000, 9_999_999,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            table,
            "encrypted_int8",
            values,
            SortDirection::Desc,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ore_order_float8() {
        trace();
        let table = "encrypted_ore_order_float8";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<f64> = vec![
            -99.9, -1.5, -0.001, 0.0, 0.001, 1.5, 3.25, 42.0, 99.9, 1000.5,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            table,
            "encrypted_float8",
            values,
            SortDirection::Asc,
        )
        .await;
    }

    #[tokio::test]
    async fn map_ore_order_float8_desc() {
        trace();
        let table = "encrypted_ore_order_float8_desc";
        clear_table(table).await;
        let client = connect_with_tls(PROXY).await;
        let values: Vec<f64> = vec![
            -99.9, -1.5, -0.001, 0.0, 0.001, 1.5, 3.25, 42.0, 99.9, 1000.5,
        ];
        ore_order_helpers::ore_order_generic(
            &client,
            table,
            "encrypted_float8",
            values,
            SortDirection::Desc,
        )
        .await;
    }
}
