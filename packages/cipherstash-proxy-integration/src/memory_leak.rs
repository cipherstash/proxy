//! Memory leak reproduction tests
//!
//! These tests replicate customer scenarios that exhibited memory leaks.
//! The primary scenario involves bulk INSERT operations through prepared statements.

#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, trace, PROXY};
    use chrono::Utc;
    use serde_json::Value;
    use serial_test::serial;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    /// Test JSON payload matching customer's credit report structure (~4KB)
    fn test_json_payload() -> Value {
        serde_json::json!({
            "reportId": "RPT-2024-001234567890",
            "generatedAt": "2024-01-15T14:30:00Z",
            "consumer": {
                "firstName": "John",
                "lastName": "Doe",
                "dateOfBirth": "1985-06-15",
                "ssn": "XXX-XX-1234",
                "addresses": [
                    {
                        "type": "current",
                        "street": "123 Main Street",
                        "city": "Springfield",
                        "state": "IL",
                        "zipCode": "62701",
                        "since": "2020-03-01"
                    },
                    {
                        "type": "previous",
                        "street": "456 Oak Avenue",
                        "city": "Chicago",
                        "state": "IL",
                        "zipCode": "60601",
                        "since": "2015-08-15"
                    }
                ],
                "employment": {
                    "employer": "Acme Corporation",
                    "position": "Software Engineer",
                    "income": 95000,
                    "since": "2019-01-15"
                }
            },
            "creditScore": {
                "value": 742,
                "model": "FICO8",
                "range": { "min": 300, "max": 850 },
                "factors": [
                    { "code": "01", "description": "Length of time accounts have been established" },
                    { "code": "14", "description": "Number of accounts with delinquency" },
                    { "code": "07", "description": "Too many inquiries last 12 months" }
                ]
            },
            "accounts": [
                {
                    "accountNumber": "XXXX-XXXX-XXXX-4567",
                    "creditor": "First National Bank",
                    "type": "creditCard",
                    "status": "open",
                    "openDate": "2018-05-20",
                    "creditLimit": 15000,
                    "balance": 3250,
                    "monthlyPayment": 150,
                    "paymentHistory": ["OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK"]
                },
                {
                    "accountNumber": "LOAN-789012",
                    "creditor": "Auto Finance LLC",
                    "type": "autoLoan",
                    "status": "open",
                    "openDate": "2022-02-10",
                    "originalAmount": 28000,
                    "balance": 18500,
                    "monthlyPayment": 485,
                    "paymentHistory": ["OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK", "OK"]
                }
            ],
            "inquiries": [
                { "date": "2024-01-10", "creditor": "Capital One", "type": "hard" },
                { "date": "2023-11-05", "creditor": "Chase Bank", "type": "hard" }
            ],
            "publicRecords": [],
            "collections": [],
            "metadata": {
                "version": "2.1.0",
                "provider": "TestProvider",
                "requestId": "REQ-2024-ABCDEF123456",
                "processingTimeMs": 245
            }
        })
    }

    /// Schema for credit_data_order_v2 table (customer's table structure)
    /// Note: Removed DEFAULT gen_random_uuid() to avoid pgcrypto dependency
    /// (tests provide explicit UUIDs anyway)
    const MEMORY_LEAK_SCHEMA: &str = r#"
        DROP TABLE IF EXISTS credit_data_order_v2;
        CREATE TABLE credit_data_order_v2 (
            id uuid PRIMARY KEY,
            created_at timestamp with time zone DEFAULT now(),
            updated_at timestamp with time zone DEFAULT now(),
            order_id uuid NOT NULL,
            account_review boolean NOT NULL DEFAULT false,
            full_report eql_v2_encrypted,
            raw_report eql_v2_encrypted,
            organization_id uuid NOT NULL
        );

        SELECT eql_v2.add_search_config(
            'credit_data_order_v2',
            'full_report',
            'ste_vec',
            'jsonb',
            '{"prefix": "credit_data_order_v2/full_report"}'
        );

        SELECT eql_v2.add_search_config(
            'credit_data_order_v2',
            'raw_report',
            'ste_vec',
            'jsonb',
            '{"prefix": "credit_data_order_v2/raw_report"}'
        );
    "#;

    /// Set up the memory leak test schema directly on the database
    async fn setup_memory_leak_schema() {
        use crate::common::{connect_with_tls, PG_PORT};

        let port = std::env::var("CS_DATABASE__PORT")
            .map(|s| s.parse().unwrap())
            .unwrap_or(PG_PORT);

        let client = connect_with_tls(port).await;
        client.simple_query(MEMORY_LEAK_SCHEMA).await.unwrap();
    }

    /// Clean up the memory leak test table
    async fn cleanup_memory_leak_table() {
        let client = connect_with_tls(PROXY).await;
        client
            .simple_query("TRUNCATE credit_data_order_v2")
            .await
            .unwrap();
    }

    /// Baseline test: single insert to verify schema and encryption work
    #[tokio::test]
    #[serial]
    async fn memory_leak_baseline_single_insert() {
        trace();
        setup_memory_leak_schema().await;
        cleanup_memory_leak_table().await;

        let client = connect_with_tls(PROXY).await;

        let id = Uuid::new_v4();
        let org_id = Uuid::parse_str("539008ae-e1ff-42ed-8a58-e3588befea9d").unwrap();
        let order_id = Uuid::new_v4();
        let json_payload = test_json_payload();
        let now = chrono::Utc::now();

        let sql = r#"
            INSERT INTO credit_data_order_v2
            (id, organization_id, order_id, account_review, full_report, raw_report, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#;

        client
            .query(
                sql,
                &[
                    &id,
                    &org_id,
                    &order_id,
                    &false,
                    &json_payload,
                    &json_payload,
                    &now,
                    &now,
                ],
            )
            .await
            .expect("Insert should succeed");

        // Verify row was inserted
        let count_sql = "SELECT COUNT(*) FROM credit_data_order_v2";
        let rows = client.query(count_sql, &[]).await.unwrap();
        let count: i64 = rows[0].get(0);
        assert_eq!(count, 1, "Should have exactly one row");
    }
}
