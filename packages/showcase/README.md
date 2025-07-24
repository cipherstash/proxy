# EQL v2 JSONB Operations Showcase

A comprehensive demonstration of EQL v2's JSONB support for searchable encryption with healthcare data.

## Table of Contents

- [Overview](#overview)
- [Database Schema](#database-schema)
- [Test Data Structure](#test-data-structure)
- [JSONB Operators](#jsonb-operators)
  - [Field Access Operators](#field-access-operators)
    - [`->` (Extract Field as JSONB)](#--extract-field-as-jsonb)
    - [`->>` (Extract Field as Text)](#---extract-field-as-text)
  - [Containment Operators](#containment-operators)
    - [`@>` (Contains)](#-contains)
    - [`<@` (Contained By)](#-contained-by)
- [JSONB Functions](#jsonb-functions)
  - [`jsonb_path_exists()`](#jsonb_path_exists)
  - [`jsonb_path_query_first()`](#jsonb_path_query_first)
  - [`jsonb_path_query()`](#jsonb_path_query)
- [Comparison Operations](#comparison-operations)
  - [Numeric Comparisons](#numeric-comparisons)
  - [String Comparisons](#string-comparisons)
  - [Date Comparisons](#date-comparisons)
  - [Float Comparisons](#float-comparisons)
- [Complex Queries](#complex-queries)
  - [JOINs with JSONB](#joins-with-jsonb)
  - [Aggregations with JSONB](#aggregations-with-jsonb)
  - [Subqueries with JSONB](#subqueries-with-jsonb)
- [Running the Showcase](#running-the-showcase)

## Overview

This showcase demonstrates EQL v2's comprehensive support for JSONB operations on encrypted data. All examples use a realistic healthcare database with encrypted patient information, showcasing how applications can query complex nested data while maintaining searchable encryption.

**Key Features:**
- ✅ All JSONB operators work with encrypted data
- ✅ Complex nested healthcare data structures
- ✅ Real-world medical scenarios (allergies, insurance, vitals)
- ✅ HIPAA-inspired data modeling with encryption
- ✅ Comprehensive test coverage of EQL capabilities

## Database Schema

The healthcare database includes:

```sql
-- Patients table with encrypted PII
CREATE TABLE patients (
    id uuid,
    pii eql_v2_encrypted,  -- Complex nested JSONB with medical data
    PRIMARY KEY(id)
);

-- EQL search configuration for patient data
SELECT eql_v2.add_search_config(
    'patients',
    'pii',
    'ste_vec',
    'jsonb',
    '{"prefix": "patients/pii"}'
);
```

## Test Data Structure

The enhanced patient data includes complex nested structures:

```json
{
  "first_name": "John",
  "last_name": "Smith",
  "email": "john.smith@email.com",
  "date_of_birth": "1985-03-15",
  "medical_history": {
    "allergies": ["penicillin", "peanuts"],
    "conditions": ["diabetes", "hypertension"],
    "emergency_contact": {
      "name": "Jane Smith",
      "phone": "+1-555-0123",
      "relationship": "spouse"
    },
    "risk_factors": {
      "cardiovascular": 75,
      "diabetes": 85,
      "overall_health": 60
    }
  },
  "insurance": {
    "provider": "HealthCorp",
    "policy_number": "HC123456",
    "group_id": 1001,
    "coverage": {
      "deductible": 500,
      "out_of_pocket_max": 3000,
      "copays": {
        "primary_care": 25,
        "specialist": 50,
        "emergency": 200
      }
    }
  },
  "vitals": {
    "height_cm": 180,
    "weight_kg": 75,
    "blood_type": "O+",
    "blood_pressure": {
      "systolic": 140,
      "diastolic": 90,
      "measured_date": "2024-01-15"
    },
    "lab_results": {
      "cholesterol": 220,
      "glucose": 95,
      "hemoglobin_a1c": 6.2,
      "test_date": "2024-01-10"
    }
  }
}
```

## JSONB Operators

### Field Access Operators

#### `->` (Extract Field as JSONB)

Extracts a field and returns it as JSONB, preserving the JSON structure.

**Test 1: Extract nested medical history**
```sql
SELECT id, pii -> 'medical_history' as medical_history
FROM patients
WHERE pii -> 'medical_history' IS NOT NULL
LIMIT 1;
```

**Test 2: Extract nested insurance information**
```sql
SELECT id, pii -> 'insurance' -> 'coverage' as coverage
FROM patients
WHERE pii -> 'insurance' ->> 'provider' = 'HealthCorp';
```

**Test 3: Extract array field**
```sql
SELECT id, pii -> 'medical_history' -> 'allergies' as allergies
FROM patients
WHERE pii -> 'medical_history' -> 'allergies' IS NOT NULL
LIMIT 1;
```

#### `->>` (Extract Field as Text)

Extracts a field and returns it as text, converting JSON values to strings.

**Test 1: Extract blood type as text**
```sql
SELECT id, pii -> 'vitals' ->> 'blood_type' as blood_type
FROM patients
WHERE pii -> 'vitals' ->> 'blood_type' = 'O+'
LIMIT 1;
```

**Test 2: Extract nested insurance provider**
```sql
SELECT id, pii -> 'insurance' ->> 'provider' as provider
FROM patients
WHERE pii -> 'insurance' ->> 'provider' = 'HealthCorp';
```

**Test 3: Chain operators for deep field access**
```sql
SELECT id, pii -> 'medical_history' -> 'emergency_contact' ->> 'name' as contact_name
FROM patients
WHERE pii -> 'medical_history' -> 'emergency_contact' ->> 'relationship' = 'spouse';
```

### Containment Operators

#### `@>` (Contains)

Tests whether the left JSONB value contains the right JSONB value.

**Test 1: Find patients with specific insurance provider**
```sql
SELECT COUNT(*) as count
FROM patients
WHERE pii @> '{"insurance": {"provider": "HealthCorp"}}';
```

**Test 2: Find patients with diabetes condition**
```sql
SELECT COUNT(*) as count
FROM patients
WHERE pii @> '{"medical_history": {"conditions": ["diabetes"]}}';
```

**Test 3: Complex nested object matching**
```sql
SELECT id, pii -> 'medical_history' -> 'emergency_contact' ->> 'name' as contact_name
FROM patients
WHERE pii @> '{"medical_history": {"emergency_contact": {"relationship": "spouse"}}}'
LIMIT 1;
```

#### `<@` (Contained By)

Tests whether the left JSONB value is contained within the right JSONB value.

**Test 1: Check if blood type structure is contained**
```sql
SELECT COUNT(*) as count
FROM patients
WHERE '{"vitals": {"blood_type": "O+"}}' <@ pii;
```

**Test 2: Verify insurance information is contained**
```sql
SELECT COUNT(*) as count
FROM patients
WHERE '{"insurance": {"group_id": 1001}}' <@ pii;
```

## JSONB Functions

### `jsonb_path_exists()`

Tests whether a JSONPath expression matches any values in the JSONB data.

**Test 1: Check if insurance coverage path exists**
```sql
SELECT COUNT(*) as count
FROM patients
WHERE jsonb_path_exists(pii, '$.insurance.coverage');
```

**Test 2: Verify medical history structure**
```sql
SELECT COUNT(*) as count
FROM patients
WHERE jsonb_path_exists(pii, '$.medical_history.risk_factors.cardiovascular');
```

**Test 3: Check for array fields**
```sql
SELECT COUNT(*) as count
FROM patients
WHERE jsonb_path_exists(pii, '$.medical_history.allergies');
```

### `jsonb_path_query_first()`

Extracts the first JSON value that matches a JSONPath expression.

**Test 1: Extract first allergy**
```sql
SELECT jsonb_path_query_first(pii, '$.medical_history.allergies[0]') as first_allergy
FROM patients
WHERE jsonb_path_exists(pii, '$.medical_history.allergies')
LIMIT 1;
```

**Test 2: Extract cardiovascular risk score**
```sql
SELECT id, jsonb_path_query_first(pii, '$.medical_history.risk_factors.cardiovascular') as cv_risk
FROM patients
WHERE CAST(jsonb_path_query_first(pii, '$.medical_history.risk_factors.cardiovascular') AS INTEGER) > 70;
```

**Test 3: Extract copay amounts**
```sql
SELECT jsonb_path_query_first(pii, '$.insurance.coverage.copays.primary_care') as primary_copay
FROM patients
WHERE jsonb_path_exists(pii, '$.insurance.coverage.copays')
LIMIT 1;
```

### `jsonb_path_query()`

Extracts all JSON values that match a JSONPath expression (returns multiple results).

**Test 1: Extract all allergies**
```sql
SELECT jsonb_path_query(pii, '$.medical_history.allergies[*]') as allergy
FROM patients
WHERE jsonb_path_exists(pii, '$.medical_history.allergies')
LIMIT 5;
```

**Test 2: Extract all conditions**
```sql
SELECT jsonb_path_query(pii, '$.medical_history.conditions[*]') as condition
FROM patients
WHERE jsonb_path_exists(pii, '$.medical_history.conditions');
```

## Comparison Operations

### Numeric Comparisons

**Test 1: Integer comparison - Find patients with high group IDs**
```sql
SELECT id, pii -> 'insurance' ->> 'group_id' as group_id
FROM patients
WHERE CAST(pii -> 'insurance' ->> 'group_id' AS INTEGER) >= 2000;
```

**Test 2: Weight comparison**
```sql
SELECT id, pii -> 'vitals' ->> 'weight_kg' as weight
FROM patients
WHERE CAST(pii -> 'vitals' ->> 'weight_kg' AS INTEGER) > 80;
```

### String Comparisons

**Test 1: Blood type pattern matching**
```sql
SELECT id, pii -> 'vitals' ->> 'blood_type' as blood_type
FROM patients
WHERE pii -> 'vitals' ->> 'blood_type' LIKE '%+';
```

**Test 2: Provider name comparison**
```sql
SELECT id, pii -> 'insurance' ->> 'provider' as provider
FROM patients
WHERE pii -> 'insurance' ->> 'provider' = 'HealthCorp';
```

### Date Comparisons

**Test 1: Recent lab results**
```sql
SELECT id, pii -> 'vitals' -> 'lab_results' ->> 'test_date' as test_date
FROM patients
WHERE pii -> 'vitals' -> 'lab_results' ->> 'test_date' >= '2024-02-01';
```

**Test 2: Blood pressure measurement dates**
```sql
SELECT id, pii -> 'vitals' -> 'blood_pressure' ->> 'measured_date' as bp_date
FROM patients
WHERE pii -> 'vitals' -> 'blood_pressure' ->> 'measured_date' >= '2024-01-01';
```

### Float Comparisons

**Test 1: Elevated A1C levels**
```sql
SELECT id, pii -> 'vitals' -> 'lab_results' ->> 'hemoglobin_a1c' as a1c
FROM patients
WHERE CAST(pii -> 'vitals' -> 'lab_results' ->> 'hemoglobin_a1c' AS FLOAT) > 6.0;
```

**Test 2: Multi-condition risk assessment**
```sql
SELECT id,
       pii -> 'vitals' ->> 'weight_kg' as weight,
       pii -> 'medical_history' -> 'risk_factors' ->> 'cardiovascular' as cv_risk
FROM patients
WHERE CAST(pii -> 'vitals' ->> 'weight_kg' AS INTEGER) > 80
  AND CAST(pii -> 'medical_history' -> 'risk_factors' ->> 'cardiovascular' AS INTEGER) > 60;
```

## Complex Queries

### JOINs with JSONB

**Test 1: Patients with specific insurance AND active prescriptions**
```sql
SELECT DISTINCT p.id,
       p.pii ->> 'first_name' as first_name,
       p.pii ->> 'last_name' as last_name,
       p.pii -> 'insurance' ->> 'provider' as insurance_provider
FROM patients p
JOIN patient_medications pm ON p.id = pm.patient_id
WHERE p.pii @> '{"insurance": {"provider": "HealthCorp"}}'
  AND pm.medication ->> 'to_date' >= '2024-01-16'
ORDER BY p.pii ->> 'last_name';
```

### Aggregations with JSONB

**Test 1: Average risk scores by insurance provider**
```sql
SELECT p.pii -> 'insurance' ->> 'provider' as provider,
       AVG(CAST(p.pii -> 'medical_history' -> 'risk_factors' ->> 'cardiovascular' AS FLOAT)) as avg_cv_risk,
       COUNT(*) as patient_count
FROM patients p
WHERE jsonb_path_exists(p.pii, '$.medical_history.risk_factors.cardiovascular')
GROUP BY p.pii -> 'insurance' ->> 'provider'
ORDER BY avg_cv_risk DESC;
```

**Test 2: Allergy statistics**
```sql
SELECT id,
       pii ->> 'first_name' as name,
       jsonb_array_length(pii -> 'medical_history' -> 'allergies') as allergy_count,
       pii -> 'insurance' -> 'coverage' ->> 'deductible' as deductible
FROM patients
WHERE jsonb_array_length(pii -> 'medical_history' -> 'allergies') > 1
  AND CAST(pii -> 'insurance' -> 'coverage' ->> 'deductible' AS INTEGER) > 500
ORDER BY allergy_count DESC;
```

### Subqueries with JSONB

**Test 1: Patients with above-average copays**
```sql
SELECT id,
       pii ->> 'first_name' as name,
       pii -> 'insurance' -> 'coverage' -> 'copays' ->> 'primary_care' as copay
FROM patients
WHERE CAST(pii -> 'insurance' -> 'coverage' -> 'copays' ->> 'primary_care' AS INTEGER) >
      (SELECT AVG(CAST(pii -> 'insurance' -> 'coverage' -> 'copays' ->> 'primary_care' AS INTEGER))
       FROM patients
       WHERE jsonb_path_exists(pii, '$.insurance.coverage.copays.primary_care'))
ORDER BY CAST(pii -> 'insurance' -> 'coverage' -> 'copays' ->> 'primary_care' AS INTEGER) DESC;
```

## Running the Showcase

### Prerequisites

1. **Start PostgreSQL with EQL**:
   ```bash
   mise run postgres:up --extra-args "--detach --wait"
   mise run postgres:setup
   ```

2. **Configure CipherStash credentials** in `mise.local.toml`:
   ```toml
   CS_WORKSPACE_CRN = "crn:region:workspace-id"
   CS_CLIENT_ACCESS_KEY = "your-access-key"
   CS_DEFAULT_KEYSET_ID = "your-keyset-id"
   CS_CLIENT_ID = "your-client-id"
   CS_CLIENT_KEY = "your-client-key"
   ```

3. **Start the CipherStash Proxy**:
   ```bash
   mise run proxy:up --extra-args "--detach --wait"
   ```

### Running the Showcase

Execute the comprehensive JSONB demonstration:

```bash
# Run the showcase
cargo run --package showcase

# Or run with mise
mise run showcase
```

### Expected Output

The showcase will execute and display:

1. **Original Healthcare Query**: Aspirin prescription lookup
2. **Field Access Operations**: Testing `->` and `->>`
3. **Containment Operations**: Testing `@>` and `<@`
4. **JSONPath Functions**: Testing `jsonb_path_*` functions
5. **Comparison Operations**: Numeric, string, date, and float comparisons
6. **Complex Nested Queries**: JOINs, aggregations, and subqueries

Each test section provides detailed output showing:
- ✅ Successful query execution
- 📊 Result counts and sample data
- 🔍 Validation of EQL's JSONB capabilities

### Key Benefits Demonstrated

- **🔒 Searchable Encryption**: All queries work on encrypted data
- **🏥 Healthcare Compliance**: HIPAA-style data protection with functionality
- **📈 Performance**: Complex queries execute efficiently with EQL
- **🔧 Developer Experience**: Standard PostgreSQL JSONB syntax works transparently
- **🛡️ Security**: Sensitive medical data remains encrypted at rest and in transit

This showcase proves that EQL v2 provides comprehensive JSONB support for encrypted data, enabling sophisticated healthcare applications while maintaining strong privacy protections.