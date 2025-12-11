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
        clear, connect_with_tls, insert, random_id, simple_query, trace, PG_LATEST, PROXY,
    };
    use serde_json::json;
    use tokio_postgres::SimpleQueryMessage;
    use tracing::info;

    const BULK_ROW_COUNT: usize = 500;
    const GIN_INDEX_NAME: &str = "encrypted_jsonb_gin_idx";

    /// Insert bulk JSONB data for GIN index testing
    ///
    /// PostgreSQL query planner needs ~500+ rows to prefer GIN index over seq scan.
    /// Each row has varied JSONB to enable meaningful containment queries.
    /// Runs ANALYZE after insert for accurate query planner statistics.
    async fn setup_bulk_jsonb_data() {
        for n in 1..=BULK_ROW_COUNT {
            let id = random_id();
            let encrypted_jsonb = json!({
                "string": format!("value_{}", n % 10),
                "number": n as i64,
                "category": format!("cat_{}", n % 5),
            });

            let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
            insert(sql, &[&id, &encrypted_jsonb]).await;
        }

        // Run ANALYZE for accurate query planner statistics
        let client = connect_with_tls(PG_LATEST).await;
        client.simple_query("ANALYZE encrypted").await.unwrap();
    }

    /// Create GIN index on encrypted_jsonb column using eql_v2.jsonb_array()
    ///
    /// Connects directly to PostgreSQL (not proxy) to create the index,
    /// then runs ANALYZE for accurate query planner statistics.
    async fn create_gin_index() {
        let client = connect_with_tls(PG_LATEST).await;

        let sql = format!(
            "CREATE INDEX IF NOT EXISTS {} ON encrypted USING GIN (eql_v2.jsonb_array(encrypted_jsonb))",
            GIN_INDEX_NAME
        );
        client.simple_query(&sql).await.unwrap();

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

    /// Check if EXPLAIN output shows usage of specified index
    fn uses_index(explain: &[String], index_name: &str) -> bool {
        explain.iter().any(|line| line.contains(index_name))
    }

    /// Check if EXPLAIN output shows sequential scan
    fn uses_seq_scan(explain: &[String]) -> bool {
        explain.iter().any(|line| line.contains("Seq Scan"))
    }

    /// Test: Verify sequential scan is used without GIN index (baseline)
    ///
    /// This establishes that the GIN index actually matters for query optimization.
    #[tokio::test]
    async fn containment_uses_seq_scan_without_index() {
        trace();
        clear().await;
        drop_gin_index().await;
        setup_bulk_jsonb_data().await;

        let sql = r#"SELECT id FROM encrypted WHERE encrypted_jsonb @> '{"string": "value_1"}'"#;
        let explain = explain_query(sql).await;

        info!("EXPLAIN output:\n{}", explain.join("\n"));

        assert!(
            uses_seq_scan(&explain),
            "Expected Seq Scan without index. EXPLAIN:\n{}",
            explain.join("\n")
        );
    }
}
