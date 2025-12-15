//! Tests for JSONB containment operators
//!
//! Verifies that the containment operator transformation works correctly:
//! - @> operator is transformed to eql_v2.jsonb_contains()
//! - eql_v2.jsonb_contains() function works with encrypted data
//! - Both return correct results matching the expected data pattern
//!
//! ## Test Data
//!
//! Uses fixture data loaded via `mise run proxy:fixtures` (500 rows with IDs 1000000-1000499).
//! Pattern: `{"string": "value_N", "number": N}` where N = n % 10
//! This gives ~50 rows per value (value_0 through value_9).

#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, trace, PROXY};
    use serde_json::json;
    use tracing::info;

    /// ID range for fixture data (loaded via mise run proxy:fixtures)
    const FIXTURE_ID_START: i64 = 1000000;
    const FIXTURE_ID_END: i64 = 1000499;
    /// Total number of fixture rows for containment tests
    const FIXTURE_COUNT: i64 = 500;

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
            // Adjust expected count based on operand types and containment semantics.
            //
            // Expected counts and variance tolerances:
            // - Subset searches (column @> search): expect ~50 rows (FIXTURE_COUNT / 10 values)
            //   Variance ±10 accounts for modulo distribution not being perfectly uniform
            // - Exact match searches (search @> column): expect 1 row, variance 0 (deterministic)
            let (expected_count, variance) = match (&lhs, &rhs) {
                // LHS is encrypted column: column @> RHS (column contains RHS)
                // Uses subset search {"string": "value_1"} - matches ~50 rows
                (OperandType::EncryptedColumn, OperandType::Parameter) => (50, 10),
                (OperandType::EncryptedColumn, OperandType::Literal) => (50, 10),
                (OperandType::EncryptedColumn, OperandType::EncryptedColumn) => (50, 10),

                // LHS is parameter: parameter @> RHS
                (OperandType::Parameter, OperandType::Parameter) => (50, 10),
                (OperandType::Parameter, OperandType::Literal) => (50, 10),
                // Parameter @> encrypted column: uses exact match search - matches 1 row
                (OperandType::Parameter, OperandType::EncryptedColumn) => (1, 0),

                // LHS is literal: literal @> RHS
                (OperandType::Literal, OperandType::Parameter) => (50, 10),
                (OperandType::Literal, OperandType::Literal) => (50, 10),
                // Literal @> encrypted column: uses exact match search - matches 1 row
                (OperandType::Literal, OperandType::EncryptedColumn) => (1, 0),
            };

            Self {
                lhs,
                rhs,
                expected_count,
                variance,
            }
        }

        /// Get the appropriate search value based on operand types
        ///
        /// For `column @> search`: use subset `{"string": "value_1"}` - matches ~50 rows
        /// For `search @> column`: use exact match `{"string": "value_1", "number": 1}` - matches 1 row
        fn search_value(&self) -> serde_json::Value {
            match (&self.lhs, &self.rhs) {
                // When searching if param/literal contains column, use exact match
                (OperandType::Parameter, OperandType::EncryptedColumn)
                | (OperandType::Literal, OperandType::EncryptedColumn) => {
                    json!({"string": "value_1", "number": 1})
                }
                // Otherwise use subset search
                _ => json!({"string": "value_1"}),
            }
        }

        /// Determine query protocol based on operand types
        fn protocol(&self) -> QueryProtocol {
            match (&self.lhs, &self.rhs) {
                (OperandType::Parameter, _) | (_, OperandType::Parameter) => {
                    QueryProtocol::Extended
                }
                _ => QueryProtocol::Simple,
            }
        }

        /// Build SQL query string based on operand types
        /// Filters by fixture ID range to isolate from other test data
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

            // Filter by fixture ID range to isolate from other test data
            format!(
                "SELECT COUNT(*) FROM encrypted WHERE {} @> {} AND id BETWEEN {} AND {}",
                lhs, rhs, FIXTURE_ID_START, FIXTURE_ID_END
            )
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
    ///
    /// Tests use fixture data in ID range FIXTURE_ID_START to FIXTURE_ID_END.
    /// Data is inserted once per test run if not already present.
    ///
    /// Search value varies by test type:
    /// - `column @> search`: subset `{"string": "value_1"}` matches ~50 rows
    /// - `search @> column`: exact `{"string": "value_1", "number": 1}` matches 1 row
    macro_rules! containment_test {
        ($name:ident, lhs = $lhs:ident, rhs = $rhs:ident) => {
            #[tokio::test]
            async fn $name() {
                trace();
                ensure_fixture_data().await;

                let client = connect_with_tls(PROXY).await;
                let test_case = ContainmentTestCase::new(OperandType::$lhs, OperandType::$rhs);
                let search_value = test_case.search_value();
                test_case.run(&client, &search_value).await;
            }
        };
    }

    /// Ensure fixture data exists in the specific ID range.
    ///
    /// Uses IDs FIXTURE_ID_START to FIXTURE_ID_END to isolate from other tests.
    /// Does NOT call clear() - preserves data from other tests.
    /// Only inserts if the fixture data is missing.
    async fn ensure_fixture_data() {
        let client = connect_with_tls(PROXY).await;

        // Check if fixture data already exists
        let sql = format!(
            "SELECT COUNT(*) FROM encrypted WHERE id BETWEEN {} AND {}",
            FIXTURE_ID_START, FIXTURE_ID_END
        );
        let rows = client.query(&sql, &[]).await.unwrap();
        let count: i64 = rows[0].get(0);

        info!(
            "Fixture records in range {}-{}: {}",
            FIXTURE_ID_START, FIXTURE_ID_END, count
        );

        if count >= FIXTURE_COUNT {
            return; // Fixture data already exists
        }

        info!("Inserting fixture data...");

        // Insert fixture rows with specific IDs
        let stmt = client
            .prepare("INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING")
            .await
            .unwrap();

        for n in 1..=FIXTURE_COUNT {
            let id = FIXTURE_ID_START + n - 1;
            let encrypted_jsonb = json!({
                "string": format!("value_{}", n % 10),
                "number": n,
            });
            client
                .execute(&stmt, &[&id, &encrypted_jsonb])
                .await
                .unwrap();
        }

        info!("Inserted {} fixture rows", FIXTURE_COUNT);
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
    /// Uses fixture data in ID range FIXTURE_ID_START to FIXTURE_ID_END.
    ///
    /// With 500 rows and "string": "value_N" where N = n % 10,
    /// we expect ~50 rows to have "string": "value_1".
    #[tokio::test]
    async fn jsonb_contains_function_works() {
        trace();
        ensure_fixture_data().await;

        let client = connect_with_tls(PROXY).await;

        // Use extended query protocol with parameterized query
        // Filter by fixture ID range to isolate from other test data
        let search_value = json!({"string": "value_1"});
        let sql = format!(
            "SELECT COUNT(*) FROM encrypted WHERE eql_v2.jsonb_contains(encrypted_jsonb, $1) AND id BETWEEN {} AND {}",
            FIXTURE_ID_START, FIXTURE_ID_END
        );

        info!("Testing eql_v2.jsonb_contains() function with SQL: {}", sql);

        let rows = client.query(&sql, &[&search_value]).await.unwrap();
        let count: i64 = rows[0].get(0);

        info!("jsonb_contains() query returned {} matching rows", count);

        // With 500 fixture rows and "string": "value_N" where N = n % 10,
        // we expect ~50 rows to have "string": "value_1"
        assert!(
            (40..=60).contains(&count), // Allow some variance
            "Expected approximately 50 rows with jsonb_contains(), got {}",
            count
        );
    }
}
