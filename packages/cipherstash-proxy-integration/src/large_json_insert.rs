//! Integration tests for large JSON payload INSERT performance.
//!
//! These tests investigate customer-reported timeout issues when inserting
//! large JSON documents into two encrypted JSONB columns.
//!
//! Customer issue (Bloom Credit):
//! - Two encrypted JSON blob columns causing 25s+ timeouts on INSERTs
//! - Blanking either column alleviates timeouts
//! - Blanking both makes insertion times negligible (milliseconds)
//!
//! Test configuration:
//! - Uses `bloom_credit` table with encrypted columns configured WITHOUT ste_vec indexes
//!
//! Test categories:
//! - **Synthetic tests** - Generated data matching customer report structure (reproducible)
//! - **Customer data tests** - Actual Bloom Credit data from `bloom_credit/` directory

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};
    use serde_json::{json, Value};
    use std::time::Instant;

    // =========================================================================
    // Configuration
    // =========================================================================

    /// Number of records to insert in each test.
    /// Adjust this to test performance at different volumes.
    const NUM_RECORDS: usize = 10;

    // =========================================================================
    // Data Generators
    // =========================================================================

    /// Generate a synthetic payment snapshot matching customer data structure.
    fn generate_payment_snapshot(month_offset: u32) -> Value {
        let statuses = ["CURRENT", "LATE_30", "LATE_60", "LATE_90"];
        let status = statuses[(month_offset as usize) % statuses.len()];
        json!({
            "occurrence_date": format!("2025-{:02}-01", (month_offset % 12) + 1),
            "payment_status": status
        })
    }

    /// Generate a synthetic tradeline matching customer data structure.
    fn generate_tradeline(index: u32) -> Value {
        let creditors = [
            "THE BANK OF MISSOURI",
            "CAPITAL ONE",
            "CITIBANK NA",
            "CHASE BANK",
            "AMERICAN EXPRESS",
            "DISCOVER BANK",
            "WELLS FARGO",
            "US BANK",
        ];
        let types = ["REVOLVING", "INSTALLMENT", "MORTGAGE", "OPEN"];
        let type_descriptions = ["Credit Card", "Auto Loan", "Home Mortgage", "Line of Credit"];

        // Generate 18 payment snapshots per tradeline
        let payment_snapshots: Vec<Value> = (0..18)
            .map(|m| generate_payment_snapshot(m + index))
            .collect();

        let creditor_name = creditors[(index as usize) % creditors.len()];
        let account_type = types[(index as usize) % types.len()];
        let account_type_desc = type_descriptions[(index as usize) % type_descriptions.len()];
        let year = (index % 5) + 1;
        let month = (index % 12) + 1;
        let day = (index % 28) + 1;
        let delinquency_date: Value = if index % 3 == 0 {
            Value::String("01092026".to_string())
        } else {
            Value::Null
        };
        let past_due = if index % 2 == 0 { (index + 1) * 10 } else { 0 };
        let terms_duration = if index % 2 == 0 { 36 } else { 60 };

        json!({
            "account": {
                "account_number": "",
                "date_closed": null,
                "date_opened": format!("202{}-{:02}-{:02}", year, month, day),
                "name": "",
                "type": account_type,
                "type_description": account_type_desc,
                "creditor": {
                    "address": null,
                    "email": null,
                    "industry": null,
                    "name": creditor_name,
                    "phone": null,
                    "website": null
                }
            },
            "account_rating": format!("{}", (index % 5) + 1),
            "credit_limit": (index + 1) * 1000,
            "current_balance": (index + 1) * 500,
            "date_effective": "2026-01-09",
            "delinquency_date": delinquency_date,
            "high_credit": (index + 1) * 800,
            "late_30_days_total": index % 3,
            "late_60_days_total": index % 4,
            "late_90_days_total": index % 5,
            "months_reviewed_count": 18,
            "most_recent_payment_amount": (index + 1) * 50,
            "most_recent_payment_date": "2025-09-01",
            "past_due": past_due,
            "payment_pattern_start_date": "2024-07-09",
            "scheduled_monthly_payment": (index + 1) * 25,
            "payment_snapshots": payment_snapshots,
            "remarks": [
                {"code": "CLO", "type": "COMPLIANCE"},
                {"code": format!("RMK{}", index), "type": "ACCOUNT"}
            ],
            "terms_duration": terms_duration,
            "terms_frequency": "MONTHLY"
        })
    }

    /// Generate a synthetic inquiry matching customer data structure.
    fn generate_inquiry(index: u32) -> Value {
        let inquirers = [
            "TD BANK USA/TARGET C",
            "CAPITAL ONE",
            "CITIBANK NA., BEST B",
            "CHASE BANK USA",
            "AMERICAN EXPRESS",
        ];
        let categories = [
            "DEPARTMENT_AND_VARIETY",
            "BANKS",
            "HOME_FURNISHINGS",
            "FINANCE_COMPANIES",
            "CREDIT_UNIONS",
        ];
        let names = [
            "Complete Dept. Stores",
            "All Banks",
            "Television & Radio",
            "Personal Finance Companies",
            "Credit Unions",
        ];

        json!({
            "date": format!("2025-{:02}-{:02}", (index % 12) + 1, (index % 28) + 1),
            "inquirer_name": inquirers[(index as usize) % inquirers.len()],
            "industry": {
                "category": categories[(index as usize) % categories.len()],
                "name": names[(index as usize) % names.len()]
            }
        })
    }

    /// Generate a synthetic credit score matching customer data structure.
    fn generate_credit_score() -> Value {
        json!({
            "model": "VANTAGE4",
            "score_reasons": [
                {"narrative": "Lack of real estate secured loan information"},
                {"narrative": "Too many bankcards with high balance compared to credit limit"},
                {"narrative": "Balances on bankcards are too high compared with credit limits"},
                {"narrative": "Length of time accounts have been established"}
            ],
            "value": 650
        })
    }

    /// Generate synthetic "full report" JSON (~250KB) matching customer structure.
    ///
    /// Structure matches Bloom Credit's full-report.json with:
    /// - 140 tradelines (each with 18 payment snapshots)
    /// - 50 inquiries
    /// - Credit scores, alerts, collections, etc.
    ///
    /// The `seed` parameter ensures each generated report is unique.
    fn generate_synthetic_full_report(seed: u32) -> Value {
        let tradelines: Vec<Value> = (0..140).map(|i| generate_tradeline(i + seed * 200)).collect();
        let inquiries: Vec<Value> = (0..50).map(|i| generate_inquiry(i + seed * 200)).collect();

        json!({
            "fullReport": {
                "credit_data_order": {
                    "order_date": "2026-01-12",
                    "order_id": "14d69ece-6e96-4bce-a8f7-ca452ff74072",
                    "alerts": [],
                    "bankruptcies": [],
                    "collections": [],
                    "consumer_id": "dc0b6c07-4f44-4b5f-9942-aeba35177a22",
                    "credit_scores": [generate_credit_score()],
                    "inquiries": inquiries,
                    "mla_statuses": [],
                    "ofac_statuses": [],
                    "tradelines": tradelines
                }
            }
        })
    }

    /// Generate a synthetic consumer record for raw report.
    fn generate_consumer_record(index: u32) -> Value {
        let cities = ["New York", "Los Angeles", "Chicago", "Houston", "Phoenix"];
        let states = ["NY", "CA", "IL", "TX", "AZ"];
        let positions = ["Manager", "Engineer", "Analyst", "Director"];

        let addresses: Vec<Value> = (0..3u32)
            .map(|a| {
                let city = cities[(a as usize) % 5];
                let state = states[(a as usize) % 5];
                let addr_type = if a == 0 { "CURRENT" } else { "PREVIOUS" };
                json!({
                    "street": format!("{} Main Street", (index * 100) + a),
                    "city": city,
                    "state": state,
                    "zip": format!("{:05}", 10000 + (index * 100) + a),
                    "type": addr_type,
                    "date_reported": format!("2025-{:02}-01", (a % 12) + 1)
                })
            })
            .collect();

        let employers: Vec<Value> = (0..2u32)
            .map(|e| {
                let position = positions[(e as usize) % 4];
                json!({
                    "name": format!("Employer {} Inc.", (index * 10) + e),
                    "address": format!("{} Business Park", (index * 10) + e),
                    "date_hired": format!("20{:02}-{:02}-01", 15 + (e % 10), (e % 12) + 1),
                    "position": position
                })
            })
            .collect();

        let phone_numbers: Vec<Value> = (0..2u32)
            .map(|p| {
                let phone_type = if p == 0 { "HOME" } else { "MOBILE" };
                json!({
                    "number": format!("555-{:03}-{:04}", 100 + (index % 900), (p * 1000) + index),
                    "type": phone_type
                })
            })
            .collect();

        let middle_name = if index % 2 == 0 {
            Value::String(format!("M{}", index))
        } else {
            Value::Null
        };

        json!({
            "consumer_id": format!("consumer-{:08}", index),
            "personal_info": {
                "first_name": format!("FirstName{}", index),
                "last_name": format!("LastName{}", index),
                "middle_name": middle_name,
                "suffix": null,
                "date_of_birth": format!("19{:02}-{:02}-{:02}", 70 + (index % 30), (index % 12) + 1, (index % 28) + 1),
                "ssn_last_four": format!("{:04}", (index * 1234) % 10000)
            },
            "addresses": addresses,
            "employers": employers,
            "phone_numbers": phone_numbers
        })
    }

    /// Generate synthetic "raw report" JSON (~500KB) matching customer structure.
    ///
    /// Structure matches Bloom Credit's raw-report.json with:
    /// - Multiple consumer records with detailed personal info
    /// - Addresses, employers, phone numbers
    /// - Additional metadata fields
    ///
    /// The `seed` parameter ensures each generated report is unique.
    fn generate_synthetic_raw_report(seed: u32) -> Value {
        let consumers: Vec<Value> = (0..270).map(|i| generate_consumer_record(i + seed * 1000)).collect();
        let consumer_count = consumers.len();

        let data_sources = ["EQUIFAX", "EXPERIAN", "TRANSUNION"];
        let statuses = ["VERIFIED", "PENDING", "FAILED"];

        // Add extra bulk data to reach ~500KB target
        let extra_data: Vec<Value> = (0..550u32)
            .map(|i| {
                let idx = i + seed * 1000;
                let source = data_sources[(idx as usize) % 3];
                let status = statuses[(idx as usize) % 3];
                let flags: Vec<String> = (0..5).map(|f| format!("FLAG_{}", f + idx)).collect();
                let day = (idx % 28) + 1;
                let hour = idx % 24;
                let minute = idx % 60;
                let response_code = 200 + (idx % 100);
                let processing_time = 50 + (idx % 500);
                let cache_hit = idx % 2 == 0;
                let retry_count = idx % 3;
                json!({
                    "record_id": format!("rec-{:08}", idx),
                    "timestamp": format!("2026-01-{:02}T{:02}:{:02}:00Z", day, hour, minute),
                    "data_source": source,
                    "processing_notes": format!("Processing note {} with additional context and details for record tracking and audit purposes. This field contains extended text to simulate realistic data volumes.", idx),
                    "verification_status": status,
                    "metadata": {
                        "request_id": format!("req-{:012}", idx * 12345),
                        "response_code": response_code,
                        "processing_time_ms": processing_time,
                        "cache_hit": cache_hit,
                        "retry_count": retry_count,
                        "flags": flags
                    }
                })
            })
            .collect();

        json!({
            "rawReport": {
                "consumers": consumers,
                "status": "COMPLETE",
                "processing_records": extra_data,
                "summary": {
                    "total_consumers": consumer_count,
                    "processing_date": "2026-01-12",
                    "report_version": "2.0",
                    "data_sources": ["EQUIFAX", "EXPERIAN", "TRANSUNION"]
                }
            }
        })
    }

    /// Load customer JSON data from bloom_credit directory.
    fn load_customer_json(filename: &str) -> Value {
        // Find the project root by looking for the bloom_credit directory
        let possible_paths = [
            format!("bloom_credit/{}", filename),
            format!("../../bloom_credit/{}", filename),
            format!("../../../bloom_credit/{}", filename),
        ];

        for path in &possible_paths {
            if let Ok(content) = std::fs::read_to_string(path) {
                return serde_json::from_str(&content)
                    .unwrap_or_else(|_| panic!("Failed to parse {} as JSON", path));
            }
        }

        // If we can't find it, give a helpful error message
        let cwd = std::env::current_dir().unwrap_or_default();
        panic!(
            "Failed to find bloom_credit/{}. Current directory: {}. Tried: {:?}",
            filename,
            cwd.display(),
            possible_paths
        )
    }

    // =========================================================================
    // Synthetic Tests (generated data - reproducible, no external dependencies)
    // =========================================================================

    /// Synthetic test - INSERT with only first encrypted JSONB column populated.
    ///
    /// Uses generated data matching customer structure (~250KB).
    /// Each record is uniquely generated.
    #[tokio::test]
    async fn synthetic_insert_first_column() {
        trace();
        clear().await;

        let client = connect_with_tls(PROXY).await;

        // Generate first record to show size
        let sample = generate_synthetic_full_report(0);
        let full_size = serde_json::to_string(&sample).unwrap().len();

        println!("Synthetic full report size: {}KB", full_size / 1024);
        println!("Inserting {} unique records...", NUM_RECORDS);

        let sql = "INSERT INTO bloom_credit (id, encrypted_jsonb_full) VALUES ($1, $2)";

        let start = Instant::now();
        for i in 0..NUM_RECORDS {
            let id = random_id();
            let full_report = generate_synthetic_full_report(i as u32);
            client.query(sql, &[&id, &full_report]).await.unwrap();
            if (i + 1) % 10 == 0 || i + 1 == NUM_RECORDS {
                println!("  Inserted {}/{} records", i + 1, NUM_RECORDS);
            }
        }
        let duration = start.elapsed();

        let avg_ms = duration.as_millis() as f64 / NUM_RECORDS as f64;
        println!(
            "SYNTHETIC INSERT first column only ({}KB): {} records in {:?} (avg: {:.1}ms/record)",
            full_size / 1024,
            NUM_RECORDS,
            duration,
            avg_ms
        );

        if avg_ms > 10000.0 {
            println!("WARNING: Average insert took >10s - in customer timeout territory!");
        }
    }

    /// Synthetic test - INSERT with BOTH encrypted JSONB columns populated.
    ///
    /// Uses generated data matching customer structure:
    /// - encrypted_jsonb_full: ~250KB
    /// - encrypted_jsonb_raw: ~500KB
    /// Each record is uniquely generated.
    #[tokio::test]
    async fn synthetic_insert_both_columns() {
        trace();
        clear().await;

        let client = connect_with_tls(PROXY).await;

        // Generate first record to show sizes
        let sample_full = generate_synthetic_full_report(0);
        let sample_raw = generate_synthetic_raw_report(0);

        let full_size = serde_json::to_string(&sample_full).unwrap().len();
        let raw_size = serde_json::to_string(&sample_raw).unwrap().len();

        println!("Synthetic full report size: {}KB", full_size / 1024);
        println!("Synthetic raw report size: {}KB", raw_size / 1024);
        println!("Inserting {} unique records...", NUM_RECORDS);

        let sql = "INSERT INTO bloom_credit (id, encrypted_jsonb_full, encrypted_jsonb_raw) VALUES ($1, $2, $3)";

        let start = Instant::now();
        for i in 0..NUM_RECORDS {
            let id = random_id();
            let full_report = generate_synthetic_full_report(i as u32);
            let raw_report = generate_synthetic_raw_report(i as u32);
            client
                .query(sql, &[&id, &full_report, &raw_report])
                .await
                .unwrap();
            if (i + 1) % 10 == 0 || i + 1 == NUM_RECORDS {
                println!("  Inserted {}/{} records", i + 1, NUM_RECORDS);
            }
        }
        let duration = start.elapsed();

        let avg_ms = duration.as_millis() as f64 / NUM_RECORDS as f64;
        println!(
            "SYNTHETIC INSERT both columns (full: {}KB, raw: {}KB): {} records in {:?} (avg: {:.1}ms/record)",
            full_size / 1024,
            raw_size / 1024,
            NUM_RECORDS,
            duration,
            avg_ms
        );

        if avg_ms > 10000.0 {
            println!("WARNING: Average insert took >10s - in customer timeout territory!");
        }
    }

    // =========================================================================
    // Customer Data Tests (actual Bloom Credit data from bloom_credit/)
    // =========================================================================

    /// Customer data test - INSERT with only first encrypted JSONB column populated.
    ///
    /// Uses actual customer data: full-report.json (~237KB)
    #[tokio::test]
    async fn customer_insert_first_column() {
        trace();
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let full_report = load_customer_json("full-report.json");
        let full_size = serde_json::to_string(&full_report).unwrap().len();

        println!("Customer full-report.json size: {}KB", full_size / 1024);
        println!("Inserting {} records...", NUM_RECORDS);

        let sql = "INSERT INTO bloom_credit (id, encrypted_jsonb_full) VALUES ($1, $2)";

        let start = Instant::now();
        for i in 0..NUM_RECORDS {
            let id = random_id();
            client.query(sql, &[&id, &full_report]).await.unwrap();
            if (i + 1) % 10 == 0 || i + 1 == NUM_RECORDS {
                println!("  Inserted {}/{} records", i + 1, NUM_RECORDS);
            }
        }
        let duration = start.elapsed();

        let avg_ms = duration.as_millis() as f64 / NUM_RECORDS as f64;
        println!(
            "CUSTOMER INSERT first column only ({}KB): {} records in {:?} (avg: {:.1}ms/record)",
            full_size / 1024,
            NUM_RECORDS,
            duration,
            avg_ms
        );

        if avg_ms > 10000.0 {
            println!("WARNING: Average insert took >10s - in customer timeout territory!");
        }
    }

    /// Customer data test - INSERT with BOTH encrypted JSONB columns populated.
    ///
    /// Uses actual customer data:
    /// - encrypted_jsonb_full: full-report.json (~237KB)
    /// - encrypted_jsonb_raw: raw-report.json (~345KB)
    ///
    /// This should reproduce the customer's timeout issue where inserts take 25s+.
    #[tokio::test]
    async fn customer_insert_both_columns() {
        trace();
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let full_report = load_customer_json("full-report.json");
        let raw_report = load_customer_json("raw-report.json");

        let full_size = serde_json::to_string(&full_report).unwrap().len();
        let raw_size = serde_json::to_string(&raw_report).unwrap().len();

        println!("Customer full-report.json size: {}KB", full_size / 1024);
        println!("Customer raw-report.json size: {}KB", raw_size / 1024);
        println!("Inserting {} records...", NUM_RECORDS);

        let sql = "INSERT INTO bloom_credit (id, encrypted_jsonb_full, encrypted_jsonb_raw) VALUES ($1, $2, $3)";

        let start = Instant::now();
        for i in 0..NUM_RECORDS {
            let id = random_id();
            client
                .query(sql, &[&id, &full_report, &raw_report])
                .await
                .unwrap();
            if (i + 1) % 10 == 0 || i + 1 == NUM_RECORDS {
                println!("  Inserted {}/{} records", i + 1, NUM_RECORDS);
            }
        }
        let duration = start.elapsed();

        let avg_ms = duration.as_millis() as f64 / NUM_RECORDS as f64;
        println!(
            "CUSTOMER INSERT both columns (full: {}KB, raw: {}KB): {} records in {:?} (avg: {:.1}ms/record)",
            full_size / 1024,
            raw_size / 1024,
            NUM_RECORDS,
            duration,
            avg_ms
        );

        if avg_ms > 10000.0 {
            println!("WARNING: Average insert took >10s - in customer timeout territory!");
        }
    }

    // =========================================================================
    // Baseline Tests
    // =========================================================================

    /// Baseline test - INSERT with both encrypted JSONB columns as NULL.
    ///
    /// This establishes the baseline timing without encryption overhead.
    #[tokio::test]
    async fn baseline_insert_both_null() {
        trace();
        clear().await;

        let client = connect_with_tls(PROXY).await;

        println!("Inserting {} records...", NUM_RECORDS);

        let sql = "INSERT INTO bloom_credit (id) VALUES ($1)";

        let start = Instant::now();
        for i in 0..NUM_RECORDS {
            let id = random_id();
            client.query(sql, &[&id]).await.unwrap();
            if (i + 1) % 10 == 0 || i + 1 == NUM_RECORDS {
                println!("  Inserted {}/{} records", i + 1, NUM_RECORDS);
            }
        }
        let duration = start.elapsed();

        let avg_ms = duration.as_millis() as f64 / NUM_RECORDS as f64;
        println!(
            "BASELINE INSERT both columns NULL: {} records in {:?} (avg: {:.1}ms/record)",
            NUM_RECORDS, duration, avg_ms
        );

        // Average should be fast (under 100ms per record for NULL inserts)
        assert!(
            avg_ms < 100.0,
            "Baseline insert average should be <100ms, got {:.1}ms",
            avg_ms
        );
    }
}
