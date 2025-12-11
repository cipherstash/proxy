//! GIN index tests for JSONB containment operations
//!
//! Tests that the new EQL containment API enables GIN index usage:
//! - eql_v2.jsonb_array() returns jsonb[] with native hash support
//! - eql_v2.jsonb_contains() / jsonb_contained_by() helper functions
//!
//! Requires 500+ rows for PostgreSQL query planner to prefer GIN index over seq scan.

#[cfg(test)]
mod tests {
    use crate::common::{
        connect_with_tls, insert, random_id, simple_query, trace, PG_LATEST, PROXY,
    };
    use serde_json::json;
    use tokio_postgres::SimpleQueryMessage;
    use tracing::info;

    const BULK_ROW_COUNT: usize = 500;
    const GIN_INDEX_NAME: &str = "encrypted_jsonb_gin_idx";

    /// Ensure bulk data exists - only insert if needed
    ///
    /// Checks row count first to avoid slow re-inserts on every test.
    /// PostgreSQL query planner needs ~500+ rows to prefer GIN index over seq scan.
    async fn ensure_bulk_data() {
        let client = connect_with_tls(PROXY).await;

        // Check current row count
        let rows = client
            .query("SELECT COUNT(*) FROM encrypted", &[])
            .await
            .unwrap();
        let count: i64 = rows[0].get(0);

        if count >= BULK_ROW_COUNT as i64 {
            return; // Data already exists
        }

        // Insert needed rows
        let stmt = client
            .prepare("INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)")
            .await
            .unwrap();

        for n in 1..=BULK_ROW_COUNT {
            let id = random_id();
            let encrypted_jsonb = json!({
                "string": format!("value_{}", n % 10),
                "number": n as i64,
            });
            client
                .execute(&stmt, &[&id, &encrypted_jsonb])
                .await
                .unwrap();
        }

        // ANALYZE for query planner
        let pg_client = connect_with_tls(PG_LATEST).await;
        pg_client.simple_query("ANALYZE encrypted").await.unwrap();
    }

    /// Create GIN index on encrypted_jsonb column using eql_v2.jsonb_array()
    ///
    /// Connects directly to PostgreSQL (not proxy) to create the index,
    /// then runs ANALYZE for accurate query planner statistics.
    /// Handles race condition when tests run in parallel.
    async fn create_gin_index() {
        let client = connect_with_tls(PG_LATEST).await;

        let sql = format!(
            "CREATE INDEX IF NOT EXISTS {} ON encrypted USING GIN (eql_v2.jsonb_array(encrypted_jsonb))",
            GIN_INDEX_NAME
        );
        // Ignore duplicate key error from parallel test execution
        let _ = client.simple_query(&sql).await;

        // ANALYZE for accurate statistics
        client.simple_query("ANALYZE encrypted").await.unwrap();
    }

    /// Drop GIN index to reset state between tests
    async fn drop_gin_index() {
        let client = connect_with_tls(PG_LATEST).await;
        let sql = format!("DROP INDEX IF EXISTS {}", GIN_INDEX_NAME);
        client.simple_query(&sql).await.unwrap();
    }

    /// Get EXPLAIN output for a query through the proxy
    ///
    /// Returns each line of EXPLAIN output as a String.
    async fn explain_query(sql: &str) -> Vec<String> {
        let client = connect_with_tls(PROXY).await;
        let explain_sql = format!("EXPLAIN {}", sql);

        let messages = client.simple_query(&explain_sql).await.unwrap();
        messages
            .iter()
            .filter_map(|msg| {
                if let SimpleQueryMessage::Row(row) = msg {
                    row.get(0).map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get EXPLAIN output for a parameterized query through the proxy
    async fn explain_query_with_params(
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Vec<String> {
        let client = connect_with_tls(PROXY).await;
        let stmt = client.prepare(&format!("EXPLAIN {}", sql)).await.unwrap();
        let rows = client.query(&stmt, params).await.unwrap();
        rows.iter()
            .filter_map(|row| row.try_get::<_, String>(0).ok())
            .collect()
    }

    /// Check if EXPLAIN output shows usage of specified index
    fn uses_index(explain: &[String], index_name: &str) -> bool {
        explain.iter().any(|line| line.contains(index_name))
    }

    /// Check if EXPLAIN output shows sequential scan
    fn uses_seq_scan(explain: &[String]) -> bool {
        explain.iter().any(|line| line.contains("Seq Scan"))
    }

    /// Test: Verify direct @> operator works through proxy and uses GIN index
    ///
    /// Tests the proxy transformation of direct @> operator to use eql_v2.jsonb_array().
    /// Uses parameterized query for comparison value.
    #[tokio::test]
    async fn jsonb_array_uses_gin_index() {
        trace();
        drop_gin_index().await;
        ensure_bulk_data().await;
        create_gin_index().await;

        // Test with direct @> operator - proxy should transform automatically
        let search_value = json!({"string": "value_1"});
        let sql = "SELECT id FROM encrypted WHERE encrypted_jsonb @> $1";

        info!("Testing @> operator transformation with SQL: {}", sql);

        let explain = explain_query_with_params(sql, &[&search_value]).await;

        info!("EXPLAIN output:\n{}", explain.join("\n"));

        assert!(
            uses_index(&explain, GIN_INDEX_NAME),
            "Expected GIN index for @> operator (transformation should have occurred). EXPLAIN:\n{}",
            explain.join("\n")
        );
    }

    /// Test: Verify eql_v2.jsonb_contains() works through proxy and uses GIN index
    ///
    /// Secondary verification: Explicit function call still works and uses GIN index.
    /// Uses parameterized query for comparison value.
    #[tokio::test]
    async fn jsonb_contains_uses_gin_index() {
        trace();
        drop_gin_index().await;
        ensure_bulk_data().await;
        create_gin_index().await;

        // Test explicit function call still works
        let search_value = json!({"string": "value_1"});
        let sql = "SELECT id FROM encrypted WHERE eql_v2.jsonb_contains(encrypted_jsonb, $1)";
        let explain = explain_query_with_params(sql, &[&search_value]).await;

        info!("EXPLAIN output:\n{}", explain.join("\n"));

        assert!(
            uses_index(&explain, GIN_INDEX_NAME),
            "Expected GIN index for eql_v2.jsonb_contains(). EXPLAIN:\n{}",
            explain.join("\n")
        );
    }

    // /// Test: Verify @> operator uses GIN index after creation
    // ///
    // /// The proxy should transform `col @> val` to use eql_v2.jsonb_array()
    // /// which enables the GIN index to be used.
    // #[tokio::test]
    // async fn jsonb_contains_operator_uses_gin_index() {
    //     trace();
    //     clear().await;
    //     drop_gin_index().await;
    //     setup_bulk_jsonb_data().await;
    //     create_gin_index().await;

    //     let sql = r#"SELECT id FROM encrypted WHERE encrypted_jsonb @> '{"string": "value_1"}'"#;
    //     let explain = explain_query(sql).await;

    //     info!("EXPLAIN output:\n{}", explain.join("\n"));

    //     assert!(
    //         uses_index(&explain, GIN_INDEX_NAME),
    //         "Expected GIN index '{}' to be used. EXPLAIN:\n{}",
    //         GIN_INDEX_NAME,
    //         explain.join("\n")
    //     );
    // }

    // /// Test: Verify <@ operator behavior with GIN index
    // ///
    // /// Tests the "contained by" direction of containment.
    // /// NOTE: PostgreSQL GIN indexes typically only efficiently support @>, not <@.
    // /// This test verifies whether the query planner uses the index for <@ queries.
    // /// If it uses seq scan, that's expected PostgreSQL behavior - document it.
    // #[tokio::test]
    // async fn jsonb_contained_by_operator_behavior_with_gin() {
    //     trace();
    //     clear().await;
    //     drop_gin_index().await;
    //     setup_bulk_jsonb_data().await;
    //     create_gin_index().await;

    //     // Test if a subset is contained in the column
    //     // '{"string": "value_1"}' <@ encrypted_jsonb means:
    //     // "find rows where {"string": "value_1"} is contained in the stored value"
    //     let sql = r#"SELECT id FROM encrypted WHERE '{"string": "value_1"}' <@ encrypted_jsonb LIMIT 10"#;
    //     let explain = explain_query(sql).await;

    //     info!("EXPLAIN output:\n{}", explain.join("\n"));

    //     // Note: GIN index may or may not be used for <@ operator
    //     // This test documents actual PostgreSQL behavior
    //     let index_used = uses_index(&explain, GIN_INDEX_NAME);
    //     let seq_scan_used = uses_seq_scan(&explain);

    //     info!(
    //         "GIN index {} for <@ operator (seq scan: {})",
    //         if index_used { "IS used" } else { "NOT used" },
    //         seq_scan_used
    //     );

    //     // Test passes either way - we're documenting behavior
    //     assert!(
    //         index_used || seq_scan_used,
    //         "Expected either index or seq scan. EXPLAIN:\n{}",
    //         explain.join("\n")
    //     );
    // }

    // /// Test: Verify eql_v2.jsonb_contains() function uses GIN index
    // ///
    // /// Tests direct usage of the EQL helper function (not operator transformation).
    // #[tokio::test]
    // async fn jsonb_contains_function_uses_gin_index() {
    //     trace();
    //     clear().await;
    //     drop_gin_index().await;
    //     setup_bulk_jsonb_data().await;
    //     create_gin_index().await;

    //     let sql = r#"SELECT id FROM encrypted WHERE eql_v2.jsonb_contains(encrypted_jsonb, '{"string": "value_1"}'::eql_v2_encrypted)"#;
    //     let explain = explain_query(sql).await;

    //     info!("EXPLAIN output:\n{}", explain.join("\n"));

    //     assert!(
    //         uses_index(&explain, GIN_INDEX_NAME),
    //         "Expected GIN index for jsonb_contains(). EXPLAIN:\n{}",
    //         explain.join("\n")
    //     );
    // }

    // /// Test: Verify eql_v2.jsonb_contained_by() function behavior with GIN index
    // ///
    // /// Tests direct usage of the EQL contained_by helper function.
    // /// NOTE: Similar to <@ operator, GIN index support for contained_by may vary.
    // #[tokio::test]
    // async fn jsonb_contained_by_function_behavior_with_gin() {
    //     trace();
    //     clear().await;
    //     drop_gin_index().await;
    //     setup_bulk_jsonb_data().await;
    //     create_gin_index().await;

    //     let sql = r#"SELECT id FROM encrypted WHERE eql_v2.jsonb_contained_by('{"string": "value_1"}'::eql_v2_encrypted, encrypted_jsonb)"#;
    //     let explain = explain_query(sql).await;

    //     info!("EXPLAIN output:\n{}", explain.join("\n"));

    //     // Document actual behavior - GIN may or may not support contained_by
    //     let index_used = uses_index(&explain, GIN_INDEX_NAME);
    //     let seq_scan_used = uses_seq_scan(&explain);

    //     info!(
    //         "GIN index {} for jsonb_contained_by() (seq scan: {})",
    //         if index_used { "IS used" } else { "NOT used" },
    //         seq_scan_used
    //     );

    //     // Test passes either way - we're documenting behavior
    //     assert!(
    //         index_used || seq_scan_used,
    //         "Expected either index or seq scan. EXPLAIN:\n{}",
    //         explain.join("\n")
    //     );
    // }

    // /// Test: Verify containment returns correct results when index is used
    // ///
    // /// Ensures GIN index doesn't break functional correctness.
    // /// With 500 rows and n % 10 pattern, expect 50 matches for "value_1".
    // #[tokio::test]
    // async fn containment_returns_correct_results_with_index() {
    //     trace();
    //     clear().await;
    //     drop_gin_index().await;
    //     setup_bulk_jsonb_data().await;
    //     create_gin_index().await;

    //     let sql = r#"SELECT COUNT(*) FROM encrypted WHERE encrypted_jsonb @> '{"string": "value_1"}'"#;
    //     let result = simple_query::<i64>(sql).await;

    //     // 500 rows with string = "value_N" where N = n % 10
    //     // So ~50 rows should have string = "value_1" (rows 1, 11, 21, ..., 491)
    //     assert_eq!(
    //         result,
    //         vec![50],
    //         "Expected 50 rows with string='value_1', got {:?}",
    //         result
    //     );
    // }
}
