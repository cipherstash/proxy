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
        -- Clear any existing EQL search configs for this table
        DELETE FROM public.eql_v2_configuration
        WHERE (data -> 'tables') ?| array['credit_data_order_v2'];

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

    /// Set up the memory leak test schema through the proxy
    /// Running through proxy ensures schema cache is updated
    async fn setup_memory_leak_schema() {
        let client = connect_with_tls(PROXY).await;
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

    /// Bulk insert test replicating customer's memory leak scenario
    ///
    /// Customer observed:
    /// - 10,000 inserts: 2.554GB memory
    /// - 1,000 more: +233MB → 2.787GB
    /// - Memory never drops
    ///
    /// This test uses:
    /// - 10 concurrent workers (matching customer's config)
    /// - 1,000 total inserts (scaled down for test speed)
    /// - Large JSON payloads (~4KB each)
    /// - Prepared statements via extended query protocol
    #[tokio::test]
    #[serial]
    async fn memory_leak_bulk_insert_concurrent() {
        trace();
        setup_memory_leak_schema().await;
        cleanup_memory_leak_table().await;

        const WORKER_COUNT: usize = 10;
        const INSERTS_PER_WORKER: usize = 100;
        const TOTAL_INSERTS: usize = WORKER_COUNT * INSERTS_PER_WORKER;

        let completed = Arc::new(AtomicUsize::new(0));
        let org_id = Uuid::parse_str("539008ae-e1ff-42ed-8a58-e3588befea9d").unwrap();

        let mut handles = Vec::with_capacity(WORKER_COUNT);

        for worker_id in 0..WORKER_COUNT {
            let completed = Arc::clone(&completed);

            let handle = tokio::spawn(async move {
                // Each worker gets its own connection (mimics connection pool)
                let client = connect_with_tls(PROXY).await;

                for i in 0..INSERTS_PER_WORKER {
                    let id = Uuid::new_v4();
                    let order_id = Uuid::new_v4();
                    let json_payload = test_json_payload();
                    let now = Utc::now();

                    // Use prepared statement (extended query protocol)
                    // This is what triggers statement accumulation in Context.statements
                    let sql = r#"
                        INSERT INTO credit_data_order_v2
                        (id, organization_id, order_id, account_review, full_report, raw_report, created_at, updated_at)
                        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    "#;

                    match client
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
                    {
                        Ok(_) => {
                            let count = completed.fetch_add(1, Ordering::SeqCst) + 1;
                            if count % 100 == 0 {
                                tracing::info!(
                                    "Progress: {}/{} inserts completed",
                                    count,
                                    TOTAL_INSERTS
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!("Worker {} insert {} failed: {}", worker_id, i, e);
                        }
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all workers to complete
        for handle in handles {
            handle.await.expect("Worker task should not panic");
        }

        let final_count = completed.load(Ordering::SeqCst);
        tracing::info!("Completed {} inserts", final_count);

        // Verify all rows were inserted
        let client = connect_with_tls(PROXY).await;
        let count_sql = "SELECT COUNT(*) FROM credit_data_order_v2";
        let rows = client.query(count_sql, &[]).await.unwrap();
        let db_count: i64 = rows[0].get(0);

        assert_eq!(
            db_count as usize, final_count,
            "Database count should match completed inserts"
        );
        assert_eq!(
            final_count, TOTAL_INSERTS,
            "All inserts should complete successfully"
        );
    }

    /// Bulk insert with explicit transactions (exact customer pattern)
    ///
    /// Customer's code does:
    /// 1. Get client from pool
    /// 2. BEGIN
    /// 3. INSERT
    /// 4. COMMIT
    /// 5. Release client
    ///
    /// This may create more prepared statements than the non-transaction version.
    #[tokio::test]
    #[serial]
    async fn memory_leak_bulk_insert_with_transactions() {
        trace();
        setup_memory_leak_schema().await;
        cleanup_memory_leak_table().await;

        const WORKER_COUNT: usize = 10;
        const INSERTS_PER_WORKER: usize = 100;
        const TOTAL_INSERTS: usize = WORKER_COUNT * INSERTS_PER_WORKER;

        let completed = Arc::new(AtomicUsize::new(0));
        let org_id = Uuid::parse_str("539008ae-e1ff-42ed-8a58-e3588befea9d").unwrap();

        let mut handles = Vec::with_capacity(WORKER_COUNT);

        for worker_id in 0..WORKER_COUNT {
            let completed = Arc::clone(&completed);

            let handle = tokio::spawn(async move {
                for i in 0..INSERTS_PER_WORKER {
                    // Get a fresh connection for each insert (mimics pool.connect/release)
                    let client = connect_with_tls(PROXY).await;

                    let id = Uuid::new_v4();
                    let order_id = Uuid::new_v4();
                    let json_payload = test_json_payload();
                    let now = Utc::now();

                    // BEGIN transaction
                    client.simple_query("BEGIN").await.unwrap();

                    let sql = r#"
                        INSERT INTO credit_data_order_v2
                        (id, organization_id, order_id, account_review, full_report, raw_report, created_at, updated_at)
                        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    "#;

                    match client
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
                    {
                        Ok(_) => {
                            // COMMIT transaction
                            client.simple_query("COMMIT").await.unwrap();

                            let count = completed.fetch_add(1, Ordering::SeqCst) + 1;
                            if count % 100 == 0 {
                                tracing::info!(
                                    "Progress: {}/{} inserts completed",
                                    count,
                                    TOTAL_INSERTS
                                );
                            }
                        }
                        Err(e) => {
                            // ROLLBACK on error
                            let _ = client.simple_query("ROLLBACK").await;
                            tracing::error!("Worker {} insert {} failed: {}", worker_id, i, e);
                        }
                    }
                    // Connection drops here (mimics client.release())
                }
            });

            handles.push(handle);
        }

        // Wait for all workers
        for handle in handles {
            handle.await.expect("Worker task should not panic");
        }

        let final_count = completed.load(Ordering::SeqCst);
        tracing::info!("Completed {} inserts with transactions", final_count);

        // Verify
        let client = connect_with_tls(PROXY).await;
        let rows = client
            .query("SELECT COUNT(*) FROM credit_data_order_v2", &[])
            .await
            .unwrap();
        let db_count: i64 = rows[0].get(0);

        assert_eq!(db_count as usize, final_count);
        assert_eq!(final_count, TOTAL_INSERTS);
    }

    /// Large-scale stress test for manual memory observation
    ///
    /// Run with: cargo test --package cipherstash-proxy-integration memory_leak_stress -- --ignored --nocapture
    ///
    /// While this runs, monitor proxy memory with:
    ///   docker stats proxy
    /// or
    ///   watch -n 1 'ps -o rss,vsz,pid,command -p $(pgrep cipherstash-proxy)'
    ///
    /// Expected behavior BEFORE fix:
    /// - Memory grows continuously
    /// - Does not drop after completion
    ///
    /// Expected behavior AFTER fix:
    /// - Memory stabilizes
    /// - May drop after completion as connections close
    #[tokio::test]
    #[serial]
    #[ignore = "Long-running stress test - run manually for memory observation"]
    async fn memory_leak_stress_10k_inserts() {
        trace();
        setup_memory_leak_schema().await;
        cleanup_memory_leak_table().await;

        const WORKER_COUNT: usize = 10;
        const INSERTS_PER_WORKER: usize = 1000;
        const TOTAL_INSERTS: usize = WORKER_COUNT * INSERTS_PER_WORKER;

        tracing::info!("Starting stress test: {} workers x {} inserts = {} total",
            WORKER_COUNT, INSERTS_PER_WORKER, TOTAL_INSERTS);
        tracing::info!("Monitor memory with: docker stats proxy");

        let completed = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(AtomicUsize::new(0));
        let org_id = Uuid::parse_str("539008ae-e1ff-42ed-8a58-e3588befea9d").unwrap();

        let start = std::time::Instant::now();
        let mut handles = Vec::with_capacity(WORKER_COUNT);

        for worker_id in 0..WORKER_COUNT {
            let completed = Arc::clone(&completed);
            let errors = Arc::clone(&errors);

            let handle = tokio::spawn(async move {
                let client = connect_with_tls(PROXY).await;

                for i in 0..INSERTS_PER_WORKER {
                    let id = Uuid::new_v4();
                    let order_id = Uuid::new_v4();
                    let json_payload = test_json_payload();
                    let now = Utc::now();

                    let sql = r#"
                        INSERT INTO credit_data_order_v2
                        (id, organization_id, order_id, account_review, full_report, raw_report, created_at, updated_at)
                        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    "#;

                    match client
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
                    {
                        Ok(_) => {
                            let count = completed.fetch_add(1, Ordering::SeqCst) + 1;
                            if count % 500 == 0 {
                                tracing::info!(
                                    "Progress: {}/{} ({:.1}%)",
                                    count,
                                    TOTAL_INSERTS,
                                    (count as f64 / TOTAL_INSERTS as f64) * 100.0
                                );
                            }
                        }
                        Err(e) => {
                            errors.fetch_add(1, Ordering::SeqCst);
                            if errors.load(Ordering::SeqCst) <= 10 {
                                tracing::error!(
                                    "Worker {} insert {} failed: {}",
                                    worker_id,
                                    i,
                                    e
                                );
                            }
                        }
                    }
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.await.expect("Worker should not panic");
        }

        let elapsed = start.elapsed();
        let final_count = completed.load(Ordering::SeqCst);
        let error_count = errors.load(Ordering::SeqCst);

        tracing::info!("Stress test complete:");
        tracing::info!("  - Completed: {} inserts", final_count);
        tracing::info!("  - Errors: {}", error_count);
        tracing::info!("  - Duration: {:.2}s", elapsed.as_secs_f64());
        tracing::info!("  - Rate: {:.0} inserts/sec", final_count as f64 / elapsed.as_secs_f64());
        tracing::info!("");
        tracing::info!("Check proxy memory now - it should not have grown significantly");
        tracing::info!("Wait 10s and check again - memory should drop as connections close");

        // Sleep to allow memory observation
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        // Verify
        let client = connect_with_tls(PROXY).await;
        let rows = client
            .query("SELECT COUNT(*) FROM credit_data_order_v2", &[])
            .await
            .unwrap();
        let db_count: i64 = rows[0].get(0);

        assert!(
            (db_count as usize) >= final_count - error_count,
            "Database should have at least {} rows, found {}",
            final_count - error_count,
            db_count
        );
    }
}
