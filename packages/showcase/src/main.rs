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

use common::{connect_with_tls, insert, reset_schema_to, table_exists, trace, PROXY};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

const SCHEMA: &str = r#"
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
"#;

/// Represents a medication in the healthcare system.
///
/// This struct contains basic information about pharmaceutical medications available for prescription.
/// The medication data is stored in plaintext as reference information that healthcare providers
/// need to search and identify medications.
#[derive(Serialize)]
struct Medication {
    /// Unique identifier for the medication (UUID)
    id: Uuid,
    /// Human-readable name of the medication (e.g., "Aspirin", "Metformin")
    name: String,
    /// Detailed description of the medication's purpose and effects
    description: String,
}

impl Medication {
    fn new(id: &str, name: &str, description: &str) -> Self {
        Self {
            id: Uuid::parse_str(id).unwrap(),
            name: name.to_string(),
            description: description.to_string(),
        }
    }
}

/// Represents a medical procedure in the healthcare system.
///
/// This struct contains reference information about medical procedures that can be performed on patients.
/// Like medications, this data is stored in plaintext for healthcare provider reference and searching.
#[derive(Serialize)]
struct Procedure {
    /// Unique identifier for the procedure (UUID)
    id: Uuid,
    /// Human-readable name of the procedure (e.g., "Blood Test", "MRI Scan")
    name: String,
    /// Detailed description of what the procedure involves
    description: String,
    /// Medical coding identifier (ICD-10-PCS, CPT, etc.)
    code: String,
    /// Category of procedure (e.g., "diagnostic", "surgical", "therapeutic")
    procedure_type: String,
}

impl Procedure {
    fn new(id: &str, name: &str, description: &str, code: &str, procedure_type: &str) -> Self {
        Self {
            id: Uuid::parse_str(id).unwrap(),
            name: name.to_string(),
            description: description.to_string(),
            code: code.to_string(),
            procedure_type: procedure_type.to_string(),
        }
    }
}

/// Represents a patient in the healthcare system.
///
/// This struct demonstrates the use of EQL v2 encryption for protecting sensitive patient data.
/// The patient's personally identifiable information (PII) is encrypted to ensure privacy and compliance
/// with healthcare regulations like HIPAA.
#[derive(Serialize)]
struct Patient {
    /// Unique identifier for the patient (UUID)
    id: Uuid,
    /// Encrypted personally identifiable information
    pii: PatientPii,
}

impl Patient {
    fn new(id: &str, first_name: &str, last_name: &str, email: &str, date_of_birth: &str) -> Self {
        Self {
            id: Uuid::parse_str(id).unwrap(),
            pii: PatientPii::new(first_name, last_name, email, date_of_birth),
        }
    }
}

/// Contains personally identifiable information for a patient.
///
/// This data is sensitive and must be encrypted when stored in the database. EQL v2 provides
/// searchable encryption, allowing healthcare providers to query patient data while maintaining
/// strong privacy protections.
#[derive(Serialize)]
struct PatientPii {
    /// Patient's first name
    first_name: String,
    /// Patient's last name
    last_name: String,
    /// Patient's email address for communication
    email: String,
    /// Patient's date of birth in ISO8601 format (YYYY-MM-DD)
    date_of_birth: String,
}

impl PatientPii {
    fn new(first_name: &str, last_name: &str, email: &str, date_of_birth: &str) -> Self {
        Self {
            first_name: first_name.to_string(),
            last_name: last_name.to_string(),
            email: email.to_string(),
            date_of_birth: date_of_birth.to_string(),
        }
    }
}

/// Represents a medication prescription for a patient.
///
/// This struct links patients to their prescribed medications and contains sensitive medical information
/// that requires encryption. The prescription details are stored using EQL v2 encryption to protect
/// patient privacy while enabling necessary medical queries.
#[derive(Serialize)]
struct Prescription {
    /// Reference to the patient receiving the prescription
    patient_id: Uuid,
    /// Encrypted prescription details
    prescription: PrescriptionDetail,
}

impl Prescription {
    fn new(
        patient_id: &str,
        medication_id: &str,
        daily_dosage: &str,
        from_date: &str,
        to_date: &str,
    ) -> Self {
        Self {
            patient_id: Uuid::parse_str(patient_id).unwrap(),
            prescription: PrescriptionDetail::new(medication_id, daily_dosage, from_date, to_date),
        }
    }
}

/// Contains detailed information about a medication prescription.
///
/// This struct holds sensitive medical information about a patient's medication regimen, including dosing and
/// duration. This data is encrypted when stored in the database.
#[derive(Serialize)]
struct PrescriptionDetail {
    /// Reference to the prescribed medication
    medication_id: Uuid,
    /// Dosage instructions (e.g., "500mg twice daily", "as needed for pain")
    daily_dosage: String,
    /// Start date of the prescription in ISO8601 format
    from_date: String,
    /// End date of the prescription in ISO8601 format
    to_date: String,
}

impl PrescriptionDetail {
    fn new(medication_id: &str, daily_dosage: &str, from_date: &str, to_date: &str) -> Self {
        Self {
            medication_id: Uuid::parse_str(medication_id).unwrap(),
            daily_dosage: daily_dosage.to_string(),
            from_date: from_date.to_string(),
            to_date: to_date.to_string(),
        }
    }
}

/// Represents a medical procedure performed on a patient.
///
/// This struct links patients to procedures they have received and contains sensitive medical information
/// about the procedure details. The procedure information is encrypted to protect patient privacy.
#[derive(Serialize)]
struct PatientProcedure {
    /// Reference to the patient who received the procedure
    patient_id: Uuid,
    /// Encrypted procedure details
    procedure: ProcedureDetail,
}

impl PatientProcedure {
    #[allow(clippy::too_many_arguments)]
    fn new(
        patient_id: &str,
        procedure_id: &str,
        when: &str,
        laterality: &str,
        body_site: &str,
        priority: &str,
        status: &str,
        preoperative_diagnosis: &str,
        post_operative_diagnosis: &str,
        procedure_outcome: &str,
    ) -> Self {
        Self {
            patient_id: Uuid::parse_str(patient_id).unwrap(),
            procedure: ProcedureDetail::new(
                procedure_id,
                when,
                laterality,
                body_site,
                priority,
                status,
                preoperative_diagnosis,
                post_operative_diagnosis,
                procedure_outcome,
            ),
        }
    }
}

/// Contains detailed information about a medical procedure performed on a patient.
///
/// This struct holds sensitive medical data about procedures, including timing, location, diagnoses,
/// and outcomes. This information is encrypted when stored in the database to protect patient privacy.
#[derive(Serialize)]
struct ProcedureDetail {
    /// Reference to the type of procedure performed
    procedure_id: Uuid,
    /// Timestamp when the procedure was performed (ISO8601 format)
    when: String,
    /// Laterality of the procedure (e.g., "left", "right", "bilateral", "not applicable")
    laterality: String,
    /// Anatomical location where the procedure was performed
    body_site: String,
    /// Priority level of the procedure (e.g., "routine", "urgent", "elective")
    priority: String,
    /// Current status of the procedure (e.g., "completed", "in progress", "cancelled")
    status: String,
    /// Clinical diagnosis or condition before the procedure was performed
    preoperative_diagnosis: String,
    /// Clinical diagnosis or findings after the procedure was completed
    post_operative_diagnosis: String,
    /// Result or outcome of the procedure (e.g., "successful", "complicated", "ongoing")
    procedure_outcome: String,
}

impl ProcedureDetail {
    #[allow(clippy::too_many_arguments)]
    fn new(
        procedure_id: &str,
        when: &str,
        laterality: &str,
        body_site: &str,
        priority: &str,
        status: &str,
        preoperative_diagnosis: &str,
        post_operative_diagnosis: &str,
        procedure_outcome: &str,
    ) -> Self {
        Self {
            procedure_id: Uuid::parse_str(procedure_id).unwrap(),
            when: when.to_string(),
            laterality: laterality.to_string(),
            body_site: body_site.to_string(),
            priority: priority.to_string(),
            status: status.to_string(),
            preoperative_diagnosis: preoperative_diagnosis.to_string(),
            post_operative_diagnosis: post_operative_diagnosis.to_string(),
            procedure_outcome: procedure_outcome.to_string(),
        }
    }
}

async fn setup_schema() {
    reset_schema_to(SCHEMA).await
}

async fn insert_test_data() {
    let medications = [
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440001",
            "Aspirin",
            "Pain reliever and anti-inflammatory medication",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440002",
            "Ibuprofen",
            "Nonsteroidal anti-inflammatory drug (NSAID)",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440003",
            "Acetaminophen",
            "Pain and fever reducer",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440004",
            "Amoxicillin",
            "Antibiotic for bacterial infections",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440005",
            "Metformin",
            "Diabetes medication that helps control blood sugar",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440006",
            "Lisinopril",
            "ACE inhibitor for high blood pressure",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440007",
            "Atorvastatin",
            "Statin medication for cholesterol management",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440008",
            "Omeprazole",
            "Proton pump inhibitor for acid reflux",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440009",
            "Losartan",
            "Angiotensin receptor blocker for hypertension",
        ),
        Medication::new(
            "550e8400-e29b-41d4-a716-446655440010",
            "Prednisone",
            "Corticosteroid for inflammation and immune suppression",
        ),
    ];

    for medication in &medications {
        let sql = "INSERT INTO medications (id, name, description) VALUES ($1, $2, $3)";
        insert(
            sql,
            &[&medication.id, &medication.name, &medication.description],
        )
        .await;
    }

    let patients = [
        Patient::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789001",
            "John",
            "Smith",
            "john.smith@email.com",
            "1985-03-15",
        ),
        Patient::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789002",
            "Sarah",
            "Johnson",
            "sarah.johnson@gmail.com",
            "1992-07-28",
        ),
        Patient::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "Michael",
            "Brown",
            "m.brown@outlook.com",
            "1978-12-03",
        ),
        Patient::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789004",
            "Emily",
            "Davis",
            "emily.davis@yahoo.com",
            "1990-09-12",
        ),
        Patient::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "Robert",
            "Wilson",
            "rob.wilson@email.com",
            "1973-11-22",
        ),
    ];

    for patient in &patients {
        let pii_json = serde_json::to_value(&patient.pii).unwrap();
        let sql = "INSERT INTO patients (id, pii) VALUES ($1, $2)";
        insert(sql, &[&patient.id, &pii_json]).await;
    }

    let procedures = [
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890001",
            "Appendectomy",
            "Surgical removal of the appendix",
            "0DT70ZZ",
            "surgical",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890002",
            "Colonoscopy",
            "Examination of the colon using a flexible tube with camera",
            "0DJ08ZZ",
            "diagnostic",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890003",
            "Blood Test",
            "Laboratory analysis of blood sample for diagnostic purposes",
            "80053",
            "diagnostic",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890004",
            "X-Ray",
            "Radiographic imaging to visualize internal structures",
            "73060",
            "imaging",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890005",
            "MRI Scan",
            "Magnetic resonance imaging for detailed internal body imaging",
            "72148",
            "imaging",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890006",
            "Biopsy",
            "Tissue sample extraction for microscopic examination",
            "10021",
            "diagnostic",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890007",
            "Echocardiogram",
            "Ultrasound examination of the heart structure and function",
            "93306",
            "diagnostic",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890008",
            "Cataract Surgery",
            "Surgical removal of clouded lens from the eye",
            "66984",
            "surgical",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890009",
            "Physical Therapy",
            "Rehabilitation treatment to restore movement and function",
            "97110",
            "therapeutic",
        ),
        Procedure::new(
            "b1c2d3e4-f5a6-4b5c-9d0e-23456789000a",
            "Endoscopy",
            "Internal examination using a flexible tube with camera",
            "43235",
            "diagnostic",
        ),
    ];

    for procedure in &procedures {
        let sql = "INSERT INTO procedures (id, name, description, code, procedure_type) VALUES ($1, $2, $3, $4, $5)";
        insert(
            sql,
            &[
                &procedure.id,
                &procedure.name,
                &procedure.description,
                &procedure.code,
                &procedure.procedure_type,
            ],
        )
        .await;
    }

    let prescriptions = [
        // Patient 1 (John Smith) - 4 prescriptions
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789001",
            "550e8400-e29b-41d4-a716-446655440001",
            "81mg once daily",
            "2024-01-15",
            "2024-12-31",
        ), // Aspirin
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789001",
            "550e8400-e29b-41d4-a716-446655440005",
            "500mg twice daily",
            "2024-02-01",
            "2024-12-31",
        ), // Metformin
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789001",
            "550e8400-e29b-41d4-a716-446655440006",
            "10mg once daily",
            "2024-03-01",
            "2024-12-31",
        ), // Lisinopril
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789001",
            "550e8400-e29b-41d4-a716-446655440007",
            "20mg once daily",
            "2024-04-01",
            "2024-12-31",
        ), // Atorvastatin
        // Patient 2 (Sarah Johnson) - 2 prescriptions
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789002",
            "550e8400-e29b-41d4-a716-446655440002",
            "200mg three times daily",
            "2024-02-15",
            "2024-05-15",
        ), // Ibuprofen
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789002",
            "550e8400-e29b-41d4-a716-446655440008",
            "20mg once daily",
            "2024-03-10",
            "2024-09-10",
        ), // Omeprazole
        // Patient 3 (Michael Brown) - 5 prescriptions
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "550e8400-e29b-41d4-a716-446655440004",
            "500mg three times daily",
            "2024-01-20",
            "2024-02-03",
        ), // Amoxicillin
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "550e8400-e29b-41d4-a716-446655440009",
            "50mg once daily",
            "2024-02-15",
            "2024-12-31",
        ), // Losartan
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "550e8400-e29b-41d4-a716-446655440003",
            "650mg as needed for pain",
            "2024-03-01",
            "2024-08-31",
        ), // Acetaminophen
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "550e8400-e29b-41d4-a716-446655440010",
            "5mg once daily",
            "2024-04-10",
            "2024-07-10",
        ), // Prednisone
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "550e8400-e29b-41d4-a716-446655440007",
            "40mg once daily",
            "2024-05-01",
            "2024-12-31",
        ), // Atorvastatin
        // Patient 4 (Emily Davis) - 3 prescriptions
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789004",
            "550e8400-e29b-41d4-a716-446655440001",
            "325mg as needed for headache",
            "2024-01-10",
            "2024-12-31",
        ), // Aspirin
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789004",
            "550e8400-e29b-41d4-a716-446655440006",
            "5mg once daily",
            "2024-02-20",
            "2024-12-31",
        ), // Lisinopril
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789004",
            "550e8400-e29b-41d4-a716-446655440005",
            "1000mg twice daily",
            "2024-03-15",
            "2024-12-31",
        ), // Metformin
        // Patient 5 (Robert Wilson) - 6 prescriptions (more than 5, but distributed across multiple conditions)
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "550e8400-e29b-41d4-a716-446655440002",
            "400mg twice daily",
            "2024-01-05",
            "2024-04-05",
        ), // Ibuprofen
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "550e8400-e29b-41d4-a716-446655440008",
            "40mg once daily",
            "2024-02-01",
            "2024-12-31",
        ), // Omeprazole
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "550e8400-e29b-41d4-a716-446655440009",
            "100mg once daily",
            "2024-03-01",
            "2024-12-31",
        ), // Losartan
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "550e8400-e29b-41d4-a716-446655440007",
            "80mg once daily",
            "2024-04-15",
            "2024-12-31",
        ), // Atorvastatin
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "550e8400-e29b-41d4-a716-446655440005",
            "850mg twice daily",
            "2024-05-01",
            "2024-12-31",
        ), // Metformin
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "550e8400-e29b-41d4-a716-446655440001",
            "81mg once daily",
            "2024-06-01",
            "2024-12-31",
        ), // Aspirin
    ];

    for prescription in &prescriptions {
        let prescription_json = serde_json::to_value(&prescription.prescription).unwrap();
        let sql = "INSERT INTO patient_medications (patient_id, medication) VALUES ($1, $2)";
        insert(sql, &[&prescription.patient_id, &prescription_json]).await;
    }

    let patient_procedures = [
        // Patient 1 (John Smith) - Has Metformin, Lisinopril, Atorvastatin (diabetes, hypertension, cholesterol)
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789001",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890003",
            "2024-02-10T09:30:00",
            "bilateral",
            "antecubital fossa",
            "routine",
            "completed",
            "diabetes mellitus type 2",
            "diabetes mellitus type 2 - controlled",
            "successful",
        ),
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789001",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890007",
            "2024-03-15T14:00:00",
            "not applicable",
            "chest",
            "routine",
            "completed",
            "hypertension",
            "mild left ventricular hypertrophy",
            "successful",
        ),
        // Patient 2 (Sarah Johnson) - Has Ibuprofen, Omeprazole (pain, acid reflux)
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789002",
            "b1c2d3e4-f5a6-4b5c-9d0e-23456789000a",
            "2024-03-05T10:15:00",
            "not applicable",
            "upper gastrointestinal tract",
            "urgent",
            "completed",
            "gastroesophageal reflux disease",
            "mild esophagitis",
            "successful",
        ),
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789002",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890004",
            "2024-02-20T11:30:00",
            "bilateral",
            "lumbar spine",
            "routine",
            "completed",
            "lower back pain",
            "mild degenerative changes L4-L5",
            "successful",
        ),
        // Patient 3 (Michael Brown) - Has Amoxicillin, Losartan, Acetaminophen, Prednisone (infection, hypertension, pain, inflammation)
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890006",
            "2024-04-02T08:45:00",
            "right",
            "shoulder",
            "elective",
            "completed",
            "suspected inflammatory arthritis",
            "rheumatoid arthritis",
            "successful",
        ),
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890005",
            "2024-04-20T16:00:00",
            "bilateral",
            "joints",
            "routine",
            "completed",
            "rheumatoid arthritis",
            "active rheumatoid arthritis with joint inflammation",
            "successful",
        ),
        // Patient 4 (Emily Davis) - Has Aspirin, Lisinopril, Metformin (cardiovascular protection, hypertension, diabetes)
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789004",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890003",
            "2024-03-25T07:30:00",
            "left",
            "antecubital fossa",
            "routine",
            "completed",
            "diabetes monitoring",
            "diabetes mellitus type 2 - well controlled",
            "successful",
        ),
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789004",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890007",
            "2024-04-10T13:15:00",
            "not applicable",
            "chest",
            "routine",
            "completed",
            "hypertension screening",
            "normal cardiac function",
            "successful",
        ),
        // Patient 5 (Robert Wilson) - Has multiple medications (complex medical history)
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890002",
            "2024-05-15T12:00:00",
            "not applicable",
            "colon",
            "routine screening",
            "completed",
            "routine colorectal screening",
            "normal colonoscopy",
            "successful",
        ),
        PatientProcedure::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789005",
            "b1c2d3e4-f5a6-4b5c-9d0e-234567890009",
            "2024-06-05T15:30:00",
            "bilateral",
            "lower extremities",
            "routine",
            "in progress",
            "mobility maintenance",
            "ongoing rehabilitation",
            "ongoing",
        ),
    ];

    for patient_procedure in &patient_procedures {
        let procedure_json = serde_json::to_value(&patient_procedure.procedure).unwrap();
        let sql = "INSERT INTO patient_procedures (patient_id, procedure) VALUES ($1, $2)";
        insert(sql, &[&patient_procedure.patient_id, &procedure_json]).await;
    }
}

async fn clear() {
    // HAZARD!
    //
    // Deleting rows from the eql_v2_configuration table is not officially supported due to the risk of data loss.
    //
    // TODO: EQL should support safe removal of config rows - at least in some kind of "test" or non-production
    // mode.
    let sql = r#"
        DELETE
          FROM public.eql_v2_configuration
          WHERE
            (data -> 'tables') ?| array[
              'patients',
              'patient_medications',
              'patient_procedures'
            ];
    "#;

    let client = connect_with_tls(PROXY).await;

    client.simple_query(sql).await.unwrap();

    let tables = &[
        "patient_medications",
        "patient_procedures",
        "patients",
        "medications",
        "procedures",
    ];

    for table in tables {
        if table_exists(table).await {
            client
                .simple_query(&format!("TRUNCATE {table} CASCADE;"))
                .await
                .unwrap();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    trace();
    clear().await;
    setup_schema().await;
    insert_test_data().await;

    let client = connect_with_tls(PROXY).await;

    // Query 1: Get the Aspirin medication ID
    let aspirin_id_sql = "SELECT id FROM medications WHERE name = 'Aspirin';";
    let rows = client.query(aspirin_id_sql, &[]).await.unwrap();
    let aspirin_id: Uuid = rows[0].get::<usize, Uuid>(0);

    // Query 2: Main parameterized query to find patients with active Aspirin prescriptions
    // Uses EQL v2 searchable encryption to query encrypted medication data
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

    // Extract email addresses from the result
    let actual_emails: Vec<Value> = rows.into_iter().map(|row| row.get(0)).collect();
    let actual_emails: Vec<String> = actual_emails
        .into_iter()
        .map(|value| serde_json::from_value(value).unwrap())
        .collect();

    println!("🩺 Healthcare Database Showcase - EQL v2 Searchable Encryption");
    println!("============================================================");
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

    // Expected patients with active Aspirin prescriptions based on test data:
    // - John Smith: "81mg once daily", from "2024-01-15" to "2024-12-31"
    // - Emily Davis: "325mg as needed for headache", from "2024-01-10" to "2024-12-31"
    // - Robert Wilson: "81mg once daily", from "2024-06-01" to "2024-12-31"
    let expected_emails = vec![
        "emily.davis@yahoo.com".to_string(),
        "john.smith@email.com".to_string(),
        "rob.wilson@email.com".to_string(),
    ];

    // Verify all expected emails are present
    for expected_email in &expected_emails {
        if !actual_emails.contains(expected_email) {
            eprintln!(
                "❌ Expected email '{}' not found in results",
                expected_email
            );
            return Err("Query validation failed".into());
        }
    }

    // Verify we have the correct number of results (no duplicates)
    if actual_emails.len() != expected_emails.len() {
        eprintln!(
            "❌ Expected {} unique emails, but got {}",
            expected_emails.len(),
            actual_emails.len()
        );
        return Err("Query validation failed".into());
    }

    // Verify ordering (emails should be sorted alphabetically)
    let mut sorted_expected = expected_emails.clone();
    sorted_expected.sort();
    if actual_emails != sorted_expected {
        eprintln!("❌ Results are not properly ordered by email address");
        return Err("Query validation failed".into());
    }

    println!();
    println!("🔒 This demonstration showcases:");
    println!("   • EQL v2 searchable encryption for sensitive patient data");
    println!("   • Healthcare-compliant database schema with proper foreign keys");
    println!("   • Realistic medical data with medications, procedures, and patient records");
    println!("   • Secure querying of encrypted data while maintaining privacy");
    println!();
    println!("✨ All query validations passed successfully!");

    Ok(())
}
