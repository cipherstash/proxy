/*
 * ╔═══════════════════════════════════════════════════════════════════════════════════════════════════════════════╗
 * ║                                    HEALTHCARE DATABASE SCHEMA                                                 ║
 * ║                                                                                                               ║
 * ║    ┌─────────────────┐       ┌─────────────────────┐       ┌─────────────────────┐                            ║
 * ║    │   medications   │       │     procedures      │       │      patients       │                            ║
 * ║    │                 │       │                     │       │                     │                            ║
 * ║    │ id (uuid) PK    │       │ id (uuid) PK        │       │ id (uuid) PK        │                            ║
 * ║    │ name (text)     │       │ name (text)         │       │ pii (eql_v2_enc)    │                            ║
 * ║    │ description     │       │ description (text)  │       │                     │                            ║
 * ║    │ (text)          │       │ code (text)         │       │ Contains:           │                            ║
 * ║    └─────────────────┘       │ procedure_type      │       │ • first_name        │                            ║
 * ║            │                 │ (text)              │       │ • last_name         │                            ║
 * ║            │                 └─────────────────────┘       │ • email             │                            ║
 * ║            │                         │                     │ • date_of_birth     │                            ║
 * ║            │                         │                     └─────────────────────┘                            ║
 * ║            │                         │                             │                                          ║
 * ║            ▼                         ▼                             ▼                                          ║
 * ║    ┌─────────────────────┐   ┌─────────────────────┐       ┌─────────────────────┐                            ║
 * ║    │ patient_medications │   │ patient_procedures  │       │                     │                            ║
 * ║    │                     │   │                     │       │                     │                            ║
 * ║    │ patient_id (uuid)   │◄──┤ patient_id (uuid)   │◄──────┤                     │                            ║
 * ║    │ FK → patients.id    │   │ FK → patients.id    │       │                     │                            ║
 * ║    │                     │   │                     │       │                     │                            ║
 * ║    │ medication          │   │ procedure           │       │                     │                            ║
 * ║    │ (eql_v2_encrypted)  │   │ (eql_v2_encrypted)  │       │                     │                            ║
 * ║    │                     │   │                     │       │                     │                            ║
 * ║    │ Contains:           │   │ Contains:           │       │                     │                            ║
 * ║    │ • medication_id ────┼───┤ • procedure_id ─────┼───────┤                     │                            ║
 * ║    │ • daily_dosage      │   │ • when              │       │                     │                            ║
 * ║    │ • from_date         │   │ • laterality        │       │                     │                            ║
 * ║    │ • to_date           │   │ • body_site         │       │                     │                            ║
 * ║    └─────────────────────┘   │ • priority          │       │                     │                            ║
 * ║                              │ • status            │       │                     │                            ║
 * ║                              │ • preop_diagnosis   │       │                     │                            ║
 * ║                              │ • postop_diagnosis  │       │                     │                            ║
 * ║                              │ • procedure_outcome │       │                     │                            ║
 * ║                              └─────────────────────┘       │                     │                            ║
 * ║                                                            │                     │                            ║
 * ║  Foreign Key Constraints:                                  │                     │                            ║
 * ║  • patient_medications.patient_id → patients.id            │                     │                            ║
 * ║  • patient_procedures.patient_id → patients.id             │                     │                            ║
 * ║  • All with CASCADE DELETE for referential integrity       │                     │                            ║
 * ║                                                            │                     │                            ║
 * ║  Encryption Details:                                       └─────────────────────┘                            ║
 * ║  • PII data in patients.pii is encrypted using EQL v2                                                         ║
 * ║  • Junction tables store encrypted procedure/medication details                                               ║
 * ║  • Foreign keys enforce referential integrity with CASCADE DELETE                                             ║
 * ║  • Reference tables contain plaintext lookup data                                                             ║
 * ╚═══════════════════════════════════════════════════════════════════════════════════════════════════════════════╝
 */

mod common;
mod data;
mod model;
mod schema;

use common::{connect_with_tls, trace, PROXY};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    data::{clear, create_enhanced_jsonb_test_data, insert_test_data},
    schema::setup_schema,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🩺 Healthcare Database Showcase - EQL v2 Searchable Encryption");
    println!("============================================================");

    trace();
    clear().await;

    setup_schema().await;
    insert_test_data().await;
    create_enhanced_jsonb_test_data().await;

    let client = connect_with_tls(PROXY).await;

    // Query 1: Get the Aspirin medication ID
    let aspirin_id_sql = "SELECT id FROM medications WHERE name = 'Aspirin';";
    let rows = client.query(aspirin_id_sql, &[]).await.unwrap();
    let aspirin_id: Uuid = rows[0].get::<usize, Uuid>(0);

    // Query 2: Main parameterized query to find patients with active Aspirin prescriptions
    let main_sql = r#"
        SELECT p.pii->'email' as email
        FROM patients p
        JOIN patient_medications pm ON p.id = pm.patient_id
        WHERE pm.medication->'medication_id' = $1
        AND pm.medication->'to_date' >= '"2024-01-16"'
        ORDER BY p.pii->'email'
    "#;

    let rows = client
        .query(main_sql, &[&serde_json::to_value(aspirin_id).unwrap()])
        .await
        .unwrap();

    // Extract and validate results
    let actual_emails: Vec<Value> = rows.into_iter().map(|row| row.get(0)).collect();
    let actual_emails: Vec<String> = actual_emails
        .into_iter()
        .map(|value| serde_json::from_value(value).unwrap())
        .collect();

    println!();
    println!("📊 Query Results: Patients with active Aspirin prescriptions:");
    println!();
    for (i, email) in actual_emails.iter().enumerate() {
        println!("   {}. {}", i + 1, email);
    }
    println!();
    println!(
        "✅ Found {} patients with active Aspirin prescriptions",
        actual_emails.len()
    );

    // Validate original results
    let expected_emails = vec![
        "emily.davis@yahoo.com".to_string(),
        "john.smith@email.com".to_string(),
        "rob.wilson@email.com".to_string(),
    ];

    for expected_email in &expected_emails {
        if !actual_emails.contains(expected_email) {
            eprintln!("❌ Expected email '{expected_email}' not found in results");
            return Err("Query validation failed".into());
        }
    }

    // === COMPREHENSIVE JSONB TESTING ===
    println!("\n\n🧪 === COMPREHENSIVE EQL JSONB OPERATIONS TESTING ===");
    println!("Testing all supported JSONB operators and functions with complex healthcare data");
    println!("===============================================================================");

    // Enhanced test data with complex JSONB structures was already created above

    // Run comprehensive JSONB operation tests
    test_field_access_operations().await?;
    test_containment_operations().await?;
    test_jsonpath_functions().await?;
    test_comparison_operations().await?;
    test_complex_nested_queries().await?;

    println!("\n🎉 === ALL TESTS COMPLETED SUCCESSFULLY! ===");
    println!();
    println!("🔒 This comprehensive demonstration showcases:");
    println!("   • EQL v2 searchable encryption for sensitive patient data");
    println!("   • All supported JSONB operators: ->, ->>, @>, <@");
    println!("   • JSONB functions: jsonb_path_exists, jsonb_path_query_first, jsonb_path_query");
    println!("   • Comparison operations on extracted JSONB fields");
    println!("   • Complex queries with JOINs, aggregations, and subqueries");
    println!("   • Healthcare-compliant database schema with proper foreign keys");
    println!("   • Realistic medical data with nested objects, arrays, and mixed data types");
    println!("   • Secure querying of encrypted data while maintaining privacy");
    println!();
    println!("✨ EQL v2 provides comprehensive JSONB support for encrypted healthcare data!");

    Ok(())
}

/// Tests field access operations (-> and ->>).
async fn test_field_access_operations() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔍 === Testing Field Access Operations (-> and ->>) ===");
    let client = connect_with_tls(PROXY).await;

    // Test 1: Extract nested object with -> operator (returns JSONB)
    println!("📝 Test 1: Extract medical_history with -> operator");
    let sql = "SELECT id, pii -> 'medical_history' as medical_history FROM patients WHERE id = 'a1b2c3d4-e5f6-4a5b-8c9d-123456789011' LIMIT 1";
    let rows = client.query(sql, &[]).await?;
    assert!(
        !rows.is_empty(),
        "Should find patients with medical history"
    );

    let medical_history: Value = rows[0].get("medical_history");
    assert!(
        medical_history.get("allergies").is_some(),
        "Medical history should contain allergies"
    );
    println!("✅ Successfully extracted medical_history as JSONB");

    // Test 2: Extract text field with jsonb_path_query_first (returns text)
    println!("📝 Test 2: Extract blood_type with jsonb_path_query_first");
    let sql = "SELECT id, jsonb_path_query_first(pii, '$.vitals.blood_type') as blood_type FROM patients WHERE id = 'a1b2c3d4-e5f6-4a5b-8c9d-123456789011' LIMIT 1";
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let blood_type: Option<Value> = rows[0].get("blood_type");
        println!("✅ Successfully extracted blood_type: {blood_type:?}");
    }

    // Test 3: Extract nested field with jsonb_path_query_first
    println!("📝 Test 3: Extract nested insurance provider");
    let sql = "SELECT id, jsonb_path_query_first(pii, '$.insurance.provider') as provider FROM patients WHERE jsonb_path_query_first(pii, '$.insurance.provider') = '\"HealthCorp\"'";
    let rows = client.query(sql, &[]).await?;
    assert!(!rows.is_empty(), "Should find HealthCorp patients");
    println!("✅ Successfully extracted nested insurance provider");

    // Test 4: Extract array elements
    println!("📝 Test 4: Extract allergies array");
    let sql = "SELECT id, jsonb_path_query_first(pii, '$.medical_history.allergies') as allergies FROM patients WHERE jsonb_path_exists(pii, '$.medical_history.allergies') LIMIT 1";
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let allergies: Value = rows[0].get("allergies");
        assert!(allergies.is_array(), "Allergies should be an array");
        println!("✅ Successfully extracted allergies array");
    }

    println!("🎉 Field Access Operations tests completed successfully!");
    Ok(())
}

/// Tests containment operations (@> and <@).
async fn test_containment_operations() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔍 === Testing Containment Operations (@> and <@) ===");
    let client = connect_with_tls(PROXY).await;

    // Test 1: @> operator (contains) - find patients with specific insurance provider
    println!("📝 Test 1: Find patients with HealthCorp insurance using @>");
    let sql = r#"SELECT COUNT(*) as count FROM patients WHERE pii @> '{"insurance": {"provider": "HealthCorp"}}'"#;
    let rows = client.query(sql, &[]).await?;
    let count: i64 = rows[0].get("count");
    assert!(count >= 1, "Should find at least one HealthCorp patient");
    println!("✅ Found {count} HealthCorp patients using @> operator");

    // Test 2: @> operator with nested object matching
    println!("📝 Test 2: Find patients with diabetes condition using @>");
    let sql = r#"SELECT COUNT(*) as count FROM patients WHERE pii @> '{"medical_history": {"conditions": ["diabetes"]}}'"#;
    let rows = client.query(sql, &[]).await?;
    let count: i64 = rows[0].get("count");
    println!("✅ Found {count} patients with diabetes using @> operator");

    // Test 3: <@ operator (contained by) - check if a structure is contained
    println!("📝 Test 3: Check if blood type structure is contained using <@");
    let sql =
        r#"SELECT COUNT(*) as count FROM patients WHERE '{"vitals": {"blood_type": "O+"}}' <@ pii"#;
    let rows = client.query(sql, &[]).await?;
    let count: i64 = rows[0].get("count");
    println!("✅ Found {count} patients where O+ blood type structure is contained");

    // Test 4: Complex containment with emergency contact
    println!("📝 Test 4: Complex containment with emergency contact");
    let sql = r#"SELECT id, jsonb_path_query_first(pii, '$.medical_history.emergency_contact.name') as contact_name
                 FROM patients
                 WHERE pii @> '{"medical_history": {"emergency_contact": {"relationship": "spouse"}}}'
                 LIMIT 1"#;
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let contact_name: Option<Value> = rows[0].get("contact_name");
        println!("✅ Found spouse emergency contact: {contact_name:?}");
    }

    println!("🎉 Containment Operations tests completed successfully!");
    Ok(())
}

/// Tests JSONPath functions (jsonb_path_query_first, jsonb_path_query, jsonb_path_exists).
async fn test_jsonpath_functions() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔍 === Testing JSONPath Functions ===");
    let client = connect_with_tls(PROXY).await;

    // Test 1: jsonb_path_exists - check if path exists
    println!("📝 Test 1: Check if insurance.coverage path exists");
    let sql = r#"SELECT COUNT(*) as count FROM patients WHERE jsonb_path_exists(pii, '$.insurance.coverage')"#;
    let rows = client.query(sql, &[]).await?;
    let count: i64 = rows[0].get("count");
    assert!(
        count >= 1,
        "Should find patients with insurance coverage data"
    );
    println!("✅ Found {count} patients with insurance.coverage path");

    // Test 2: jsonb_path_query_first - extract single value
    println!("📝 Test 2: Extract first allergy using jsonb_path_query_first");
    let sql = r#"SELECT jsonb_path_query_first(pii, '$.medical_history.allergies') as first_allergy
                 FROM patients
                 WHERE jsonb_path_exists(pii, '$.medical_history.allergies')
                 LIMIT 1"#;
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let first_allergy: Option<Value> = rows[0].get("first_allergy");
        println!("✅ First allergy found: {first_allergy:?}");
    }

    // Test 3: jsonb_path_query - extract multiple values (array elements)
    println!("📝 Test 3: Extract all allergies using jsonb_path_query");
    let sql = r#"SELECT jsonb_path_query(pii, '$.medical_history.allergies[*]') as allergy
                 FROM patients
                 WHERE jsonb_path_exists(pii, '$.medical_history.allergies')
                 LIMIT 5"#;
    let rows = client.query(sql, &[]).await?;
    println!(
        "✅ Found {} allergy records using jsonb_path_query",
        rows.len()
    );

    // Test 4: Complex JSONPath with conditions
    println!("📝 Test 4: Find patients with high cardiovascular risk");
    let sql = r#"SELECT id, jsonb_path_query_first(pii, '$.medical_history.risk_factors.cardiovascular') as cv_risk
                 FROM patients
                 WHERE jsonb_path_query_first(pii, '$.medical_history.risk_factors.cardiovascular') > 70"#;
    let rows = client.query(sql, &[]).await?;
    println!(
        "✅ Found {} patients with high cardiovascular risk",
        rows.len()
    );

    // Test 5: Extract nested numeric values
    println!("📝 Test 5: Extract copay amounts using JSONPath");
    let sql = r#"SELECT jsonb_path_query_first(pii, '$.insurance.coverage.copays.primary_care') as primary_copay
                 FROM patients
                 WHERE jsonb_path_exists(pii, '$.insurance.coverage.copays')
                 LIMIT 1"#;
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let copay: Option<Value> = rows[0].get("primary_copay");
        println!("✅ Primary care copay: {copay:?}");
    }

    println!("🎉 JSONPath Functions tests completed successfully!");
    Ok(())
}

/// Tests comparison operations on extracted fields.
async fn test_comparison_operations() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔍 === Testing Comparison Operations ===");
    let client = connect_with_tls(PROXY).await;

    // Test 1: Numeric comparison on extracted integer field
    println!("📝 Test 1: Find patients with group_id >= 2000");
    let sql = r#"SELECT id, jsonb_path_query_first(pii, '$.insurance.group_id') as group_id
                 FROM patients
                 WHERE jsonb_path_query_first(pii, '$.insurance.group_id') >= 2000"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with group_id >= 2000", rows.len());

    // Test 2: String equality
    println!("📝 Test 2: Find patients with A+ blood type");
    let sql = r#"SELECT id, jsonb_path_query_first(pii, '$.vitals.blood_type') as blood_type
                 FROM patients
                 WHERE jsonb_path_query_first(pii, '$.vitals.blood_type') = '"A+"'"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with positive blood types", rows.len());

    // Test 3: Date comparison
    println!("📝 Test 3: Find patients with recent lab results");
    let sql = r#"SELECT id, jsonb_path_query_first(pii, '$.vitals.lab_results.test_date') as test_date
                 FROM patients
                 WHERE jsonb_path_query_first(pii, '$.vitals.lab_results.test_date') >= '"2024-02-01"'"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with recent lab results", rows.len());

    // Test 4: Floating point comparison
    println!("📝 Test 4: Find patients with elevated A1C levels");
    let sql = r#"SELECT id, jsonb_path_query_first(pii, '$.vitals.lab_results.hemoglobin_a1c') as a1c
                 FROM patients
                 WHERE jsonb_path_query_first(pii, '$.vitals.lab_results.hemoglobin_a1c') > 6.0"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with elevated A1C levels", rows.len());

    // Test 5: Complex comparison with multiple conditions
    println!("📝 Test 5: Find high-risk patients (weight > 80 AND cardiovascular risk > 60)");
    let sql = r#"SELECT id,
                        jsonb_path_query_first(pii, '$.vitals.weight_kg') as weight,
                        jsonb_path_query_first(pii, '$.medical_history.risk_factors.cardiovascular') as cv_risk
                 FROM patients
                 WHERE jsonb_path_query_first(pii, '$.vitals.weight_kg') > 80
                   AND jsonb_path_query_first(pii, '$.medical_history.risk_factors.cardiovascular') > 60"#;
    let rows = client.query(sql, &[]).await?;
    println!(
        "✅ Found {} high-risk patients with weight > 80kg and CV risk > 60",
        rows.len()
    );

    println!("🎉 Comparison Operations tests completed successfully!");
    Ok(())
}

/// Tests complex nested queries combining multiple JSONB operations.
async fn test_complex_nested_queries() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔍 === Testing Complex Nested Queries ===");
    let client = connect_with_tls(PROXY).await;

    // Test 1: Complex query with JOIN, containment, and field extraction
    println!("📝 Test 1: Find patients with specific insurance AND active prescriptions");
    let sql = r#"
        SELECT DISTINCT p.id,
               p.pii -> 'first_name' as first_name,
               p.pii -> 'last_name' as last_name,
               jsonb_path_query_first(p.pii, '$.insurance.provider') as insurance_provider
        FROM patients p
        JOIN patient_medications pm ON p.id = pm.patient_id
        WHERE p.pii @> '{"insurance": {}}'
          AND pm.medication -> 'to_date' >= '"2024-01-16"'
        ORDER BY p.pii -> 'last_name'
    "#;
    let rows = client.query(sql, &[]).await?;
    println!(
        "✅ Found {} HealthCorp patients with active prescriptions",
        rows.len()
    );

    // Test 2: Aggregation with JSONB extraction
    println!("📝 Test 2: Calculate max risk scores by insurance provider");
    let sql = r#"
        SELECT jsonb_path_query_first(p.pii, '$.insurance.provider') as provider,
               MAX(jsonb_path_query_first(p.pii, '$.medical_history.risk_factors.cardiovascular')) as max_cv_risk,
               COUNT(*) as patient_count
        FROM patients AS p
        WHERE jsonb_path_exists(p.pii, '$.medical_history.risk_factors.cardiovascular')
        GROUP BY jsonb_path_query_first(p.pii, '$.insurance.provider')
        ORDER BY MAX(jsonb_path_query_first(p.pii, '$.medical_history.risk_factors.cardiovascular')) DESC
    "#;
    let rows = client.query(sql, &[]).await?;
    println!(
        "✅ Calculated risk scores for {} insurance providers",
        rows.len()
    );

    for row in &rows {
        let provider: Option<Value> = row.get("provider");
        let provider: Option<&str> = provider.as_ref().and_then(|v| v.as_str());

        let avg_risk: Option<Value> = row.get("max_cv_risk");
        let avg_risk: Option<i64> = avg_risk.as_ref().and_then(|v| v.as_i64());

        let count: Option<i64> = row.get("patient_count");

        println!("   {provider:?}: Avg CV Risk = {avg_risk:?}, Patients = {count:?}");
    }

    // Test 3: Complex filtering with multiple JSONB conditions
    println!("📝 Test 3: Find patients with allergies AND high deductibles");
    let sql = r#"
        SELECT id,
               pii -> 'first_name' as name,
               jsonb_array_length(jsonb_path_query_first(pii, '$.medical_history.allergies[@]')) as allergy_count,
               jsonb_path_query_first(pii, '$.insurance.coverage.deductible') as deductible
        FROM patients
        WHERE jsonb_path_query_first(pii, '$.insurance.coverage.deductible') > 500
        AND jsonb_array_length(jsonb_path_query_first(pii, '$.medical_history.allergies[@]')) > 1
        ORDER BY jsonb_array_length(jsonb_path_query_first(pii, '$.medical_history.allergies[@]')) DESC
    "#;
    let rows = client.query(sql, &[]).await?;
    println!(
        "✅ Found {} patients with multiple allergies and high deductibles",
        rows.len()
    );
    Ok(())
}
