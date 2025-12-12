//! Tests for JSONB containment operators
//!
//! Verifies that the containment operator transformation works correctly:
//! - @> operator is transformed to eql_v2.jsonb_contains()
//! - eql_v2.jsonb_contains() function works with encrypted data
//! - Both return correct results matching the expected data pattern

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};
    use serde_json::json;
    use tracing::{info, warn};

    /// Operand type for containment operator tests
    #[derive(Debug, Clone, Copy)]
    enum OperandType {
        /// Table column reference: `encrypted_jsonb`
        EncryptedColumn,
        /// Parameterized value: `$1`
        Parameter,
        /// JSON literal in SQL: `'{"key":"value"}'`
        Literal,
    }

    /// Query protocol - determines how the query is executed
    #[derive(Debug, Clone, Copy)]
    enum QueryProtocol {
        /// Extended query protocol with parameters (client.query with params)
        Extended,
        /// Simple query protocol with SQL string only
        Simple,
    }

    /// Configuration for a single containment operator test
    struct ContainmentTestCase {
        /// Left-hand side operand type
        lhs: OperandType,
        /// Right-hand side operand type
        rhs: OperandType,
        /// Expected row count (approximately)
        expected_count: i64,
        /// Allowed variance for count assertion
        variance: i64,
    }

    impl ContainmentTestCase {
        fn new(lhs: OperandType, rhs: OperandType) -> Self {
            Self {
                lhs,
                rhs,
                expected_count: 50,
                variance: 10,
            }
        }

        /// Determine query protocol based on operand types
        fn protocol(&self) -> QueryProtocol {
            match (&self.lhs, &self.rhs) {
                (OperandType::Parameter, _) | (_, OperandType::Parameter) => QueryProtocol::Extended,
                _ => QueryProtocol::Simple,
            }
        }
    }

    const BULK_ROW_COUNT: usize = 500;

    /// Ensure bulk data exists - only insert if needed
    ///
    /// Checks row count first to avoid slow re-inserts on every test.
    /// Inserts 500 rows with `"string": format!("value_{}", n % 10)` pattern.
    async fn ensure_bulk_data() {
        let client = connect_with_tls(PROXY).await;
        clear().await;

        // Check current row count
        let rows = client
            .query("SELECT COUNT(*) FROM encrypted", &[])
            .await
            .unwrap();
        let count: i64 = rows[0].get(0);

        info!("Records: {count}");

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

        info!("Inserted {} rows for testing", BULK_ROW_COUNT);
    }

    /// Test: Verify direct @> operator transformation works through proxy
    ///
    /// Tests that the proxy transforms direct @> operator to eql_v2.jsonb_contains().
    /// Runs the actual query (not EXPLAIN) to verify transformation works end-to-end.
    ///
    /// With 500 rows and "string": "value_N" where N = n % 10,
    /// we expect ~50 rows to have "string": "value_1" (rows 1, 11, 21, ..., 491).
    #[tokio::test]
    async fn jsonb_containment_operator_transformation() {
        trace();
        ensure_bulk_data().await;

        let client = connect_with_tls(PROXY).await;

        // Use extended query protocol with parameterized query
        // The @> operator should be transformed to eql_v2.jsonb_contains()
        let search_value = json!({"string": "value_1"});
        let sql = "SELECT COUNT(*) FROM encrypted WHERE encrypted_jsonb @> $1";

        info!("Testing @> operator transformation with SQL: {}", sql);

        let rows = client.query(sql, &[&search_value]).await.unwrap();
        let count: i64 = rows[0].get(0);

        info!("Containment query returned {} matching rows", count);

        // With 500 rows and "string": "value_N" where N = n % 10,
        // we expect ~50 rows to have "string": "value_1"
        assert!(
            count >= 40 && count <= 60, // Allow some variance
            "Expected approximately 50 rows with @> containment, got {}",
            count
        );
    }

    /// Test: Verify eql_v2.jsonb_contains() function works through proxy
    ///
    /// Tests explicit eql_v2.jsonb_contains() function call works correctly.
    ///
    /// With 500 rows and "string": "value_N" where N = n % 10,
    /// we expect ~50 rows to have "string": "value_1" (rows 1, 11, 21, ..., 491).
    #[tokio::test]
    async fn jsonb_contains_function_works() {
        trace();
        ensure_bulk_data().await;

        let client = connect_with_tls(PROXY).await;

        // Use extended query protocol with parameterized query
        let search_value = json!({"string": "value_1"});
        let sql = "SELECT COUNT(*) FROM encrypted WHERE eql_v2.jsonb_contains(encrypted_jsonb, $1)";

        info!("Testing eql_v2.jsonb_contains() function with SQL: {}", sql);

        let rows = client.query(sql, &[&search_value]).await.unwrap();
        let count: i64 = rows[0].get(0);

        info!("jsonb_contains() query returned {} matching rows", count);

        // With 500 rows and "string": "value_N" where N = n % 10,
        // we expect ~50 rows to have "string": "value_1"
        assert!(
            count >= 40 && count <= 60, // Allow some variance
            "Expected approximately 50 rows with jsonb_contains(), got {}",
            count
        );
    }
}
