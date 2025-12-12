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
            // Adjust expected count based on operand types and containment semantics
            let (expected_count, variance) = match (&lhs, &rhs) {
                // LHS is encrypted column: column @> RHS (column contains RHS)
                (OperandType::EncryptedColumn, OperandType::Parameter) => (50, 10),
                (OperandType::EncryptedColumn, OperandType::Literal) => (50, 10),
                (OperandType::EncryptedColumn, OperandType::EncryptedColumn) => (50, 10),

                // LHS is parameter: parameter @> RHS
                (OperandType::Parameter, OperandType::Parameter) => (50, 10),
                (OperandType::Parameter, OperandType::Literal) => (50, 10),
                // Parameter contains individual encrypted column values is generally false
                (OperandType::Parameter, OperandType::EncryptedColumn) => (0, 1),

                // LHS is literal: literal @> RHS
                (OperandType::Literal, OperandType::Parameter) => (50, 10),
                (OperandType::Literal, OperandType::Literal) => (50, 10),
                // Literal contains individual encrypted column values is generally false
                (OperandType::Literal, OperandType::EncryptedColumn) => (0, 1),
            };

            Self {
                lhs,
                rhs,
                expected_count,
                variance,
            }
        }

        /// Determine query protocol based on operand types
        fn protocol(&self) -> QueryProtocol {
            match (&self.lhs, &self.rhs) {
                (OperandType::Parameter, _) | (_, OperandType::Parameter) => QueryProtocol::Extended,
                _ => QueryProtocol::Simple,
            }
        }

        /// Build SQL query string based on operand types
        fn build_sql(&self, search_json: &serde_json::Value) -> String {
            let lhs = match self.lhs {
                OperandType::EncryptedColumn => "encrypted_jsonb".to_string(),
                OperandType::Parameter => "$1".to_string(),
                OperandType::Literal => format!("'{}'", search_json),
            };

            let rhs = match self.rhs {
                OperandType::EncryptedColumn => "encrypted_jsonb".to_string(),
                OperandType::Parameter => {
                    // If LHS is also a parameter, this is $2
                    if matches!(self.lhs, OperandType::Parameter) {
                        "$2".to_string()
                    } else {
                        "$1".to_string()
                    }
                }
                OperandType::Literal => format!("'{}'", search_json),
            };

            format!("SELECT COUNT(*) FROM encrypted WHERE {} @> {}", lhs, rhs)
        }

        /// Execute the test case
        async fn run(&self, client: &tokio_postgres::Client, search_json: &serde_json::Value) {
            let sql = self.build_sql(search_json);
            info!("Testing @> with LHS={:?}, RHS={:?}", self.lhs, self.rhs);
            info!("SQL: {}", sql);

            let count: i64 = match self.protocol() {
                QueryProtocol::Extended => {
                    let rows = client.query(&sql, &[search_json]).await.unwrap();
                    rows[0].get(0)
                }
                QueryProtocol::Simple => {
                    let rows = client.simple_query(&sql).await.unwrap();
                    // Find the first Row message (there may be RowDescription and CommandComplete messages)
                    rows.iter()
                        .find_map(|msg| {
                            if let tokio_postgres::SimpleQueryMessage::Row(row) = msg {
                                row.get(0).map(|v| v.parse::<i64>().unwrap())
                            } else {
                                None
                            }
                        })
                        .expect("No Row message found in simple_query response")
                }
            };

            info!("Result count: {}", count);

            let min = self.expected_count - self.variance;
            let max = self.expected_count + self.variance;
            assert!(
                count >= min && count <= max,
                "@> with LHS={:?}, RHS={:?}: expected {}-{} rows, got {}",
                self.lhs,
                self.rhs,
                min,
                max,
                count
            );
        }
    }

    /// Generate a containment operator test from operand types
    macro_rules! containment_test {
        ($name:ident, lhs = $lhs:ident, rhs = $rhs:ident) => {
            #[tokio::test]
            async fn $name() {
                trace();
                ensure_bulk_data().await;

                let client = connect_with_tls(PROXY).await;
                let search_value = json!({"string": "value_1"});

                let test_case = ContainmentTestCase::new(
                    OperandType::$lhs,
                    OperandType::$rhs,
                );
                test_case.run(&client, &search_value).await;
            }
        };
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

    // ============================================================================
    // @> Containment Operator Tests via Macro
    // ============================================================================

    // Encrypted column @> parameter (extended protocol)
    containment_test!(
        encrypted_contains_param,
        lhs = EncryptedColumn,
        rhs = Parameter
    );

    // Encrypted column @> literal (simple protocol)
    containment_test!(
        encrypted_contains_literal,
        lhs = EncryptedColumn,
        rhs = Literal
    );

    // Parameter @> encrypted column (extended protocol)
    containment_test!(
        param_contains_encrypted,
        lhs = Parameter,
        rhs = EncryptedColumn
    );

    // Literal @> encrypted column (simple protocol)
    containment_test!(
        literal_contains_encrypted,
        lhs = Literal,
        rhs = EncryptedColumn
    );

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
