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

/// Enhanced patient PII with complex JSONB medical metadata for comprehensive testing.
///
/// This extended structure demonstrates EQL v2's capabilities with complex nested JSONB data,
/// including arrays, nested objects, and mixed data types commonly found in healthcare systems.
#[derive(Serialize)]
struct EnhancedPatientPii {
    /// Patient's first name
    first_name: String,
    /// Patient's last name
    last_name: String,
    /// Patient's email address for communication
    email: String,
    /// Patient's date of birth in ISO8601 format (YYYY-MM-DD)
    date_of_birth: String,
    /// Complex medical history metadata
    medical_history: MedicalHistory,
    /// Insurance information
    insurance: InsuranceInfo,
    /// Current vital signs and measurements
    vitals: VitalSigns,
}

/// Medical history information containing arrays and nested data.
#[derive(Serialize)]
struct MedicalHistory {
    /// Known allergies as array of strings
    allergies: Vec<String>,
    /// Chronic conditions as array of strings
    conditions: Vec<String>,
    /// Emergency contact information
    emergency_contact: EmergencyContact,
    /// Risk factors with numeric scores
    risk_factors: RiskFactors,
}

/// Emergency contact details.
#[derive(Serialize)]
struct EmergencyContact {
    /// Contact person's name
    name: String,
    /// Contact phone number
    phone: String,
    /// Relationship to patient
    relationship: String,
}

/// Risk assessment scores.
#[derive(Serialize)]
struct RiskFactors {
    /// Cardiovascular risk score (0-100)
    cardiovascular: i32,
    /// Diabetes risk score (0-100)
    diabetes: i32,
    /// Overall health score (0-100)
    overall_health: i32,
}

/// Insurance provider information.
#[derive(Serialize)]
struct InsuranceInfo {
    /// Insurance provider name
    provider: String,
    /// Policy number
    policy_number: String,
    /// Group ID for employer plans
    group_id: i32,
    /// Coverage details
    coverage: CoverageDetails,
}

/// Insurance coverage breakdown.
#[derive(Serialize)]
struct CoverageDetails {
    /// Deductible amount in dollars
    deductible: i32,
    /// Out-of-pocket maximum
    out_of_pocket_max: i32,
    /// Copay amounts for different services
    copays: CopayInfo,
}

/// Copay information for different medical services.
#[derive(Serialize)]
struct CopayInfo {
    /// Primary care visit copay
    primary_care: i32,
    /// Specialist visit copay
    specialist: i32,
    /// Emergency room copay
    emergency: i32,
}

/// Current vital signs and physical measurements.
#[derive(Serialize)]
struct VitalSigns {
    /// Height in centimeters
    height_cm: i32,
    /// Weight in kilograms
    weight_kg: i32,
    /// Blood type (A+, A-, B+, B-, AB+, AB-, O+, O-)
    blood_type: String,
    /// Blood pressure readings
    blood_pressure: BloodPressure,
    /// Recent lab results
    lab_results: LabResults,
}

/// Blood pressure measurements.
#[derive(Serialize)]
struct BloodPressure {
    /// Systolic pressure
    systolic: i32,
    /// Diastolic pressure
    diastolic: i32,
    /// Date of measurement
    measured_date: String,
}

/// Laboratory test results.
#[derive(Serialize)]
struct LabResults {
    /// Cholesterol level (mg/dL)
    cholesterol: i32,
    /// Blood glucose level (mg/dL)
    glucose: i32,
    /// Hemoglobin A1C percentage
    hemoglobin_a1c: f32,
    /// Date of lab work
    test_date: String,
}

/// Enhanced patient with complex JSONB metadata.
#[derive(Serialize)]
struct EnhancedPatient {
    /// Unique identifier for the patient (UUID)
    id: Uuid,
    /// Enhanced PII with complex medical metadata
    pii: EnhancedPatientPii,
}

impl EnhancedPatient {
    fn new(
        id: &str,
        first_name: &str,
        last_name: &str,
        email: &str,
        date_of_birth: &str,
        pii_data: EnhancedPatientPii,
    ) -> Self {
        Self {
            id: Uuid::parse_str(id).unwrap(),
            pii: pii_data,
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

    // === ORIGINAL SHOWCASE: Aspirin Query ===
    println!("🩺 Healthcare Database Showcase - EQL v2 Searchable Encryption");
    println!("============================================================");

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
    println!("✅ Found {} patients with active Aspirin prescriptions", actual_emails.len());

    // Validate original results
    let expected_emails = vec![
        "emily.davis@yahoo.com".to_string(),
        "john.smith@email.com".to_string(),
        "rob.wilson@email.com".to_string(),
    ];

    for expected_email in &expected_emails {
        if !actual_emails.contains(expected_email) {
            eprintln!("❌ Expected email '{}' not found in results", expected_email);
            return Err("Query validation failed".into());
        }
    }

    // === COMPREHENSIVE JSONB TESTING ===
    println!("\n\n🧪 === COMPREHENSIVE EQL JSONB OPERATIONS TESTING ===");
    println!("Testing all supported JSONB operators and functions with complex healthcare data");
    println!("===============================================================================");

    // Create enhanced test data with complex JSONB structures
    create_enhanced_jsonb_test_data().await;

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

/// Creates enhanced JSONB test data with complex nested medical information.
async fn create_enhanced_jsonb_test_data() {
    println!("📋 Creating enhanced JSONB test data...");

    let enhanced_patients = [
        // Patient 1: John Smith with complex medical data
        EnhancedPatient::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789001",
            "John",
            "Smith",
            "john.smith@email.com",
            "1985-03-15",
            EnhancedPatientPii {
                first_name: "John".to_string(),
                last_name: "Smith".to_string(),
                email: "john.smith@email.com".to_string(),
                date_of_birth: "1985-03-15".to_string(),
                medical_history: MedicalHistory {
                    allergies: vec!["penicillin".to_string(), "peanuts".to_string()],
                    conditions: vec!["diabetes".to_string(), "hypertension".to_string()],
                    emergency_contact: EmergencyContact {
                        name: "Jane Smith".to_string(),
                        phone: "+1-555-0123".to_string(),
                        relationship: "spouse".to_string(),
                    },
                    risk_factors: RiskFactors {
                        cardiovascular: 75,
                        diabetes: 85,
                        overall_health: 60,
                    },
                },
                insurance: InsuranceInfo {
                    provider: "HealthCorp".to_string(),
                    policy_number: "HC123456".to_string(),
                    group_id: 1001,
                    coverage: CoverageDetails {
                        deductible: 500,
                        out_of_pocket_max: 3000,
                        copays: CopayInfo {
                            primary_care: 25,
                            specialist: 50,
                            emergency: 200,
                        },
                    },
                },
                vitals: VitalSigns {
                    height_cm: 180,
                    weight_kg: 75,
                    blood_type: "O+".to_string(),
                    blood_pressure: BloodPressure {
                        systolic: 140,
                        diastolic: 90,
                        measured_date: "2024-01-15".to_string(),
                    },
                    lab_results: LabResults {
                        cholesterol: 220,
                        glucose: 95,
                        hemoglobin_a1c: 6.2,
                        test_date: "2024-01-10".to_string(),
                    },
                },
            },
        ),
        // Patient 2: Sarah Johnson with different insurance
        EnhancedPatient::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789002",
            "Sarah",
            "Johnson",
            "sarah.johnson@gmail.com",
            "1992-07-28",
            EnhancedPatientPii {
                first_name: "Sarah".to_string(),
                last_name: "Johnson".to_string(),
                email: "sarah.johnson@gmail.com".to_string(),
                date_of_birth: "1992-07-28".to_string(),
                medical_history: MedicalHistory {
                    allergies: vec!["shellfish".to_string()],
                    conditions: vec!["asthma".to_string()],
                    emergency_contact: EmergencyContact {
                        name: "Robert Johnson".to_string(),
                        phone: "+1-555-0456".to_string(),
                        relationship: "father".to_string(),
                    },
                    risk_factors: RiskFactors {
                        cardiovascular: 25,
                        diabetes: 15,
                        overall_health: 85,
                    },
                },
                insurance: InsuranceInfo {
                    provider: "BlueCross".to_string(),
                    policy_number: "BC789012".to_string(),
                    group_id: 2002,
                    coverage: CoverageDetails {
                        deductible: 1000,
                        out_of_pocket_max: 5000,
                        copays: CopayInfo {
                            primary_care: 20,
                            specialist: 40,
                            emergency: 150,
                        },
                    },
                },
                vitals: VitalSigns {
                    height_cm: 165,
                    weight_kg: 58,
                    blood_type: "A+".to_string(),
                    blood_pressure: BloodPressure {
                        systolic: 115,
                        diastolic: 75,
                        measured_date: "2024-02-10".to_string(),
                    },
                    lab_results: LabResults {
                        cholesterol: 185,
                        glucose: 82,
                        hemoglobin_a1c: 5.1,
                        test_date: "2024-02-05".to_string(),
                    },
                },
            },
        ),
        // Patient 3: Michael Brown with high risk factors
        EnhancedPatient::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789003",
            "Michael",
            "Brown",
            "m.brown@outlook.com",
            "1978-12-03",
            EnhancedPatientPii {
                first_name: "Michael".to_string(),
                last_name: "Brown".to_string(),
                email: "m.brown@outlook.com".to_string(),
                date_of_birth: "1978-12-03".to_string(),
                medical_history: MedicalHistory {
                    allergies: vec!["latex".to_string(), "sulfa".to_string(), "iodine".to_string()],
                    conditions: vec!["hypertension".to_string(), "high_cholesterol".to_string()],
                    emergency_contact: EmergencyContact {
                        name: "Lisa Brown".to_string(),
                        phone: "+1-555-0789".to_string(),
                        relationship: "wife".to_string(),
                    },
                    risk_factors: RiskFactors {
                        cardiovascular: 90,
                        diabetes: 65,
                        overall_health: 45,
                    },
                },
                insurance: InsuranceInfo {
                    provider: "HealthCorp".to_string(),
                    policy_number: "HC345678".to_string(),
                    group_id: 1001,
                    coverage: CoverageDetails {
                        deductible: 750,
                        out_of_pocket_max: 4000,
                        copays: CopayInfo {
                            primary_care: 30,
                            specialist: 60,
                            emergency: 250,
                        },
                    },
                },
                vitals: VitalSigns {
                    height_cm: 175,
                    weight_kg: 95,
                    blood_type: "B-".to_string(),
                    blood_pressure: BloodPressure {
                        systolic: 160,
                        diastolic: 100,
                        measured_date: "2024-03-01".to_string(),
                    },
                    lab_results: LabResults {
                        cholesterol: 280,
                        glucose: 110,
                        hemoglobin_a1c: 6.8,
                        test_date: "2024-02-25".to_string(),
                    },
                },
            },
        ),
    ];

    for patient in &enhanced_patients {
        let pii_json = serde_json::to_value(&patient.pii).unwrap();
        let sql = "INSERT INTO patients (id, pii) VALUES ($1, $2)";
        insert(sql, &[&patient.id, &pii_json]).await;
    }

    println!("✅ Enhanced JSONB test data created successfully");
}

/// Tests field access operations (-> and ->>).
async fn test_field_access_operations() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔍 === Testing Field Access Operations (-> and ->>) ===");
    let client = connect_with_tls(PROXY).await;

    // Test 1: Extract nested object with -> operator (returns JSONB)
    println!("📝 Test 1: Extract medical_history with -> operator");
    let sql = "SELECT id, pii -> 'medical_history' as medical_history FROM patients WHERE pii -> 'medical_history' IS NOT NULL LIMIT 1";
    let rows = client.query(sql, &[]).await?;
    assert!(!rows.is_empty(), "Should find patients with medical history");

    let medical_history: Value = rows[0].get("medical_history");
    assert!(medical_history.get("allergies").is_some(), "Medical history should contain allergies");
    println!("✅ Successfully extracted medical_history as JSONB");

    // Test 2: Extract text field with ->> operator (returns text)
    println!("📝 Test 2: Extract blood_type with ->> operator");
    let sql = "SELECT id, pii -> 'vitals' ->> 'blood_type' as blood_type FROM patients WHERE pii -> 'vitals' ->> 'blood_type' = 'O+' LIMIT 1";
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let blood_type: Option<String> = rows[0].get("blood_type");
        println!("✅ Successfully extracted blood_type: {:?}", blood_type);
    }

    // Test 3: Extract nested field with chained operators
    println!("📝 Test 3: Extract nested insurance provider");
    let sql = "SELECT id, pii -> 'insurance' ->> 'provider' as provider FROM patients WHERE pii -> 'insurance' ->> 'provider' = 'HealthCorp'";
    let rows = client.query(sql, &[]).await?;
    assert!(!rows.is_empty(), "Should find HealthCorp patients");
    println!("✅ Successfully extracted nested insurance provider");

    // Test 4: Extract array elements
    println!("📝 Test 4: Extract allergies array");
    let sql = "SELECT id, pii -> 'medical_history' -> 'allergies' as allergies FROM patients WHERE pii -> 'medical_history' -> 'allergies' IS NOT NULL LIMIT 1";
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
    println!("✅ Found {} HealthCorp patients using @> operator", count);

    // Test 2: @> operator with nested object matching
    println!("📝 Test 2: Find patients with diabetes condition using @>");
    let sql = r#"SELECT COUNT(*) as count FROM patients WHERE pii @> '{"medical_history": {"conditions": ["diabetes"]}}'"#;
    let rows = client.query(sql, &[]).await?;
    let count: i64 = rows[0].get("count");
    println!("✅ Found {} patients with diabetes using @> operator", count);

    // Test 3: <@ operator (contained by) - check if a structure is contained
    println!("📝 Test 3: Check if blood type structure is contained using <@");
    let sql = r#"SELECT COUNT(*) as count FROM patients WHERE '{"vitals": {"blood_type": "O+"}}' <@ pii"#;
    let rows = client.query(sql, &[]).await?;
    let count: i64 = rows[0].get("count");
    println!("✅ Found {} patients where O+ blood type structure is contained", count);

    // Test 4: Complex containment with emergency contact
    println!("📝 Test 4: Complex containment with emergency contact");
    let sql = r#"SELECT id, pii -> 'medical_history' -> 'emergency_contact' ->> 'name' as contact_name
                 FROM patients
                 WHERE pii @> '{"medical_history": {"emergency_contact": {"relationship": "spouse"}}}'
                 LIMIT 1"#;
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let contact_name: Option<String> = rows[0].get("contact_name");
        println!("✅ Found spouse emergency contact: {:?}", contact_name);
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
    assert!(count >= 1, "Should find patients with insurance coverage data");
    println!("✅ Found {} patients with insurance.coverage path", count);

    // Test 2: jsonb_path_query_first - extract single value
    println!("📝 Test 2: Extract first allergy using jsonb_path_query_first");
    let sql = r#"SELECT jsonb_path_query_first(pii, '$.medical_history.allergies[0]') as first_allergy
                 FROM patients
                 WHERE jsonb_path_exists(pii, '$.medical_history.allergies')
                 LIMIT 1"#;
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let first_allergy: Option<Value> = rows[0].get("first_allergy");
        println!("✅ First allergy found: {:?}", first_allergy);
    }

    // Test 3: jsonb_path_query - extract multiple values (array elements)
    println!("📝 Test 3: Extract all allergies using jsonb_path_query");
    let sql = r#"SELECT jsonb_path_query(pii, '$.medical_history.allergies[*]') as allergy
                 FROM patients
                 WHERE jsonb_path_exists(pii, '$.medical_history.allergies')
                 LIMIT 5"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} allergy records using jsonb_path_query", rows.len());

    // Test 4: Complex JSONPath with conditions
    println!("📝 Test 4: Find patients with high cardiovascular risk");
    let sql = r#"SELECT id, jsonb_path_query_first(pii, '$.medical_history.risk_factors.cardiovascular') as cv_risk
                 FROM patients
                 WHERE CAST(jsonb_path_query_first(pii, '$.medical_history.risk_factors.cardiovascular') AS INTEGER) > 70"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with high cardiovascular risk", rows.len());

    // Test 5: Extract nested numeric values
    println!("📝 Test 5: Extract copay amounts using JSONPath");
    let sql = r#"SELECT jsonb_path_query_first(pii, '$.insurance.coverage.copays.primary_care') as primary_copay
                 FROM patients
                 WHERE jsonb_path_exists(pii, '$.insurance.coverage.copays')
                 LIMIT 1"#;
    let rows = client.query(sql, &[]).await?;
    if !rows.is_empty() {
        let copay: Option<Value> = rows[0].get("primary_copay");
        println!("✅ Primary care copay: {:?}", copay);
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
    let sql = r#"SELECT id, pii -> 'insurance' ->> 'group_id' as group_id
                 FROM patients
                 WHERE CAST(pii -> 'insurance' ->> 'group_id' AS INTEGER) >= 2000"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with group_id >= 2000", rows.len());

    // Test 2: String comparison
    println!("📝 Test 2: Find patients with blood type containing '+'");
    let sql = r#"SELECT id, pii -> 'vitals' ->> 'blood_type' as blood_type
                 FROM patients
                 WHERE pii -> 'vitals' ->> 'blood_type' LIKE '%+'"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with positive blood types", rows.len());

    // Test 3: Date comparison
    println!("📝 Test 3: Find patients with recent lab results");
    let sql = r#"SELECT id, pii -> 'vitals' -> 'lab_results' ->> 'test_date' as test_date
                 FROM patients
                 WHERE pii -> 'vitals' -> 'lab_results' ->> 'test_date' >= '2024-02-01'"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with recent lab results", rows.len());

    // Test 4: Floating point comparison
    println!("📝 Test 4: Find patients with elevated A1C levels");
    let sql = r#"SELECT id, pii -> 'vitals' -> 'lab_results' ->> 'hemoglobin_a1c' as a1c
                 FROM patients
                 WHERE CAST(pii -> 'vitals' -> 'lab_results' ->> 'hemoglobin_a1c' AS FLOAT) > 6.0"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with elevated A1C levels", rows.len());

    // Test 5: Complex comparison with multiple conditions
    println!("📝 Test 5: Find high-risk patients (weight > 80 AND cardiovascular risk > 60)");
    let sql = r#"SELECT id,
                        pii -> 'vitals' ->> 'weight_kg' as weight,
                        pii -> 'medical_history' -> 'risk_factors' ->> 'cardiovascular' as cv_risk
                 FROM patients
                 WHERE CAST(pii -> 'vitals' ->> 'weight_kg' AS INTEGER) > 80
                   AND CAST(pii -> 'medical_history' -> 'risk_factors' ->> 'cardiovascular' AS INTEGER) > 60"#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} high-risk patients with weight > 80kg and CV risk > 60", rows.len());

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
               p.pii ->> 'first_name' as first_name,
               p.pii ->> 'last_name' as last_name,
               p.pii -> 'insurance' ->> 'provider' as insurance_provider
        FROM patients p
        JOIN patient_medications pm ON p.id = pm.patient_id
        WHERE p.pii @> '{"insurance": {"provider": "HealthCorp"}}'
          AND pm.medication ->> 'to_date' >= '2024-01-16'
        ORDER BY p.pii ->> 'last_name'
    "#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} HealthCorp patients with active prescriptions", rows.len());

    // Test 2: Aggregation with JSONB extraction
    println!("📝 Test 2: Calculate average risk scores by insurance provider");
    let sql = r#"
        SELECT p.pii -> 'insurance' ->> 'provider' as provider,
               AVG(CAST(p.pii -> 'medical_history' -> 'risk_factors' ->> 'cardiovascular' AS FLOAT)) as avg_cv_risk,
               COUNT(*) as patient_count
        FROM patients p
        WHERE jsonb_path_exists(p.pii, '$.medical_history.risk_factors.cardiovascular')
        GROUP BY p.pii -> 'insurance' ->> 'provider'
        ORDER BY avg_cv_risk DESC
    "#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Calculated risk scores for {} insurance providers", rows.len());

    for row in &rows {
        let provider: Option<String> = row.get("provider");
        let avg_risk: Option<f64> = row.get("avg_cv_risk");
        let count: Option<i64> = row.get("patient_count");
        println!("   {:?}: Avg CV Risk = {:?}, Patients = {:?}", provider, avg_risk, count);
    }

    // Test 3: Complex filtering with multiple JSONB conditions
    println!("📝 Test 3: Find patients with allergies AND high deductibles");
    let sql = r#"
        SELECT id,
               pii ->> 'first_name' as name,
               jsonb_array_length(pii -> 'medical_history' -> 'allergies') as allergy_count,
               pii -> 'insurance' -> 'coverage' ->> 'deductible' as deductible
        FROM patients
        WHERE jsonb_array_length(pii -> 'medical_history' -> 'allergies') > 1
          AND CAST(pii -> 'insurance' -> 'coverage' ->> 'deductible' AS INTEGER) > 500
        ORDER BY allergy_count DESC
    "#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with multiple allergies and high deductibles", rows.len());

    // Test 4: Subquery with JSONB operations
    println!("📝 Test 4: Find patients with above-average copays");
    let sql = r#"
        SELECT id,
               pii ->> 'first_name' as name,
               pii -> 'insurance' -> 'coverage' -> 'copays' ->> 'primary_care' as copay
        FROM patients
        WHERE CAST(pii -> 'insurance' -> 'coverage' -> 'copays' ->> 'primary_care' AS INTEGER) >
              (SELECT AVG(CAST(pii -> 'insurance' -> 'coverage' -> 'copays' ->> 'primary_care' AS INTEGER))
               FROM patients
               WHERE jsonb_path_exists(pii, '$.insurance.coverage.copays.primary_care'))
        ORDER BY CAST(pii -> 'insurance' -> 'coverage' -> 'copays' ->> 'primary_care' AS INTEGER) DESC
    "#;
    let rows = client.query(sql, &[]).await?;
    println!("✅ Found {} patients with above-average copays", rows.len());

    println!("🎉 Complex Nested Queries tests completed successfully!");
    Ok(())
}

