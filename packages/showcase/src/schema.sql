  -- Patients table with encrypted PII
  DROP TABLE IF EXISTS patients CASCADE;
  CREATE TABLE patients (
      id uuid,
      pii eql_v2_encrypted,
      PRIMARY KEY(id)
  );

  SELECT eql_v2.add_search_config(
      'patients',
      'pii',
      'ste_vec',
      'jsonb',
      '{"prefix": "patients/pii"}'
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
      medication eql_v2_encrypted,
      FOREIGN KEY (patient_id) REFERENCES patients(id) ON DELETE CASCADE
  );

  SELECT eql_v2.add_search_config(
      'patient_medications',
      'medication',
      'ste_vec',
      'jsonb',
      '{"prefix": "patient_medications/medication"}'
  );

  -- Patient procedures junction table with encrypted details
  DROP TABLE IF EXISTS patient_procedures CASCADE;
  CREATE TABLE patient_procedures (
      patient_id uuid,
      procedure eql_v2_encrypted,
      FOREIGN KEY (patient_id) REFERENCES patients(id) ON DELETE CASCADE
  );

  SELECT eql_v2.add_search_config(
      'patient_procedures',
      'procedure',
      'ste_vec',
      'jsonb',
      '{"prefix": "patient_procedures/procedure"}'
  );