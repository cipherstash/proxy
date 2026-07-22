-- Patients table with encrypted PII.
--
-- EQL v3 uses self-configuring domain types: `eql_v3_json_search` is the
-- searchable encrypted-JSON (SteVec) domain, so the column type alone declares
-- the encryption and its searchability — there is no separate
-- `eql_v2.add_search_config` call as in EQL v2.
DROP TABLE IF EXISTS patients CASCADE;
CREATE TABLE patients (
    id uuid,
    pii eql_v3_json_search,
    PRIMARY KEY(id)
);

-- Medications reference table (plaintext)
DROP TABLE IF EXISTS medications CASCADE;
CREATE TABLE medications (
    id uuid,
    name text,
    description text,
    PRIMARY KEY(id)
);

-- Procedures reference table (plaintext)
DROP TABLE IF EXISTS procedures CASCADE;
CREATE TABLE procedures (
    id uuid,
    name text,
    description text,
    code text,
    procedure_type text,
    PRIMARY KEY(id)
);

-- Patient medications junction table with encrypted details
DROP TABLE IF EXISTS patient_medications CASCADE;
CREATE TABLE patient_medications (
    patient_id uuid,
    medication eql_v3_json_search,
    FOREIGN KEY (patient_id) REFERENCES patients(id) ON DELETE CASCADE
);

-- Patient procedures junction table with encrypted details
DROP TABLE IF EXISTS patient_procedures CASCADE;
CREATE TABLE patient_procedures (
    patient_id uuid,
    procedure eql_v3_json_search,
    FOREIGN KEY (patient_id) REFERENCES patients(id) ON DELETE CASCADE
);
