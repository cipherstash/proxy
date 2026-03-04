/// Multitenant ORE ordering tests.
///
/// Verifies that ORDER BY works correctly on encrypted columns for each tenant keyset.
/// The default keyset (`CS_DEFAULT_KEYSET_ID`) is already covered by `map_ore_index_order.rs`
/// and is unset during multitenant test execution.
///
/// Uses a macro to generate all 18 ORE ordering tests for each of 3 tenant keysets (54 total).
#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, trace, PROXY};
    use crate::ore_order_helpers;
    use crate::ore_order_helpers::SortDirection;

    /// Connect to the proxy and set the tenant keyset.
    ///
    /// Validates `keyset_id` as a UUID before issuing the SET command.
    async fn connect_as_tenant(keyset_id: &str) -> tokio_postgres::Client {
        uuid::Uuid::parse_str(keyset_id)
            .unwrap_or_else(|_| panic!("invalid UUID for keyset_id: {keyset_id}"));
        let client = connect_with_tls(PROXY).await;
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{keyset_id}'");
        client.execute(&sql, &[]).await.unwrap();
        client
    }

    /// Read a keyset ID from the environment, panicking with a descriptive message.
    fn keyset_id(env_var: &str) -> String {
        std::env::var(env_var)
            .unwrap_or_else(|_| panic!("{env_var} must be set for multitenant ORE tests"))
    }

    /// Generates a submodule with all 18 ORE ordering tests for a given tenant keyset.
    macro_rules! ore_order_tests_for_tenant {
        ($mod_name:ident, $env_var:expr) => {
            mod $mod_name {
                use super::*;
                use serial_test::serial;

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_text() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_text(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_text_desc() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_text_desc(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_nulls_last_by_default() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_nulls_last_by_default(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_nulls_first() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_nulls_first(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_qualified_column() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_qualified_column(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_qualified_column_with_alias() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_qualified_column_with_alias(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_no_eql_column_in_select_projection() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_no_eql_column_in_select_projection(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_can_order_by_plaintext_column() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_plaintext_column(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_can_order_by_plaintext_and_eql_columns() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_plaintext_and_eql_columns(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_simple_protocol() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    ore_order_helpers::ore_order_simple_protocol(&client).await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_int2() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    let values: Vec<i16> = vec![-100, -10, -1, 0, 1, 5, 10, 20, 100, 200];
                    ore_order_helpers::ore_order_generic(
                        &client,
                        "encrypted_int2",
                        values,
                        SortDirection::Asc,
                    )
                    .await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_int2_desc() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
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
                async fn multitenant_ore_order_int4() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    let values: Vec<i32> = vec![
                        -50_000, -1_000, -1, 0, 1, 42, 1_000, 10_000, 50_000, 100_000,
                    ];
                    ore_order_helpers::ore_order_generic(
                        &client,
                        "encrypted_int4",
                        values,
                        SortDirection::Asc,
                    )
                    .await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_int4_desc() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
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
                async fn multitenant_ore_order_int8() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
                    let values: Vec<i64> = vec![
                        -1_000_000, -10_000, -1, 0, 1, 42, 10_000, 100_000, 1_000_000, 9_999_999,
                    ];
                    ore_order_helpers::ore_order_generic(
                        &client,
                        "encrypted_int8",
                        values,
                        SortDirection::Asc,
                    )
                    .await;
                }

                #[tokio::test]
                #[serial]
                async fn multitenant_ore_order_int8_desc() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
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
                async fn multitenant_ore_order_float8() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
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
                async fn multitenant_ore_order_float8_desc() {
                    trace();
                    clear().await;
                    let client = connect_as_tenant(&keyset_id($env_var)).await;
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
        };
    }

    ore_order_tests_for_tenant!(tenant1, "CS_TENANT_KEYSET_ID_1");
    ore_order_tests_for_tenant!(tenant2, "CS_TENANT_KEYSET_ID_2");
    ore_order_tests_for_tenant!(tenant3, "CS_TENANT_KEYSET_ID_3");
}
