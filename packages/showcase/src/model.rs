
use uuid::Uuid;
use serde::Serialize;

/// Represents a medication in the healthcare system.
///
/// This struct contains basic information about pharmaceutical medications available for prescription.
/// The medication data is stored in plaintext as reference information that healthcare providers
/// need to search and identify medications.
#[derive(Serialize)]
pub struct Medication {
    /// Unique identifier for the medication (UUID)
    pub id: Uuid,
    /// Human-readable name of the medication (e.g., "Aspirin", "Metformin")
    pub name: String,
    /// Detailed description of the medication's purpose and effects
    pub description: String,
}

impl Medication {
    pub fn new(id: &str, name: &str, description: &str) -> Self {
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
pub struct Procedure {
    /// Unique identifier for the procedure (UUID)
    pub id: Uuid,
    /// Human-readable name of the procedure (e.g., "Blood Test", "MRI Scan")
    pub name: String,
    /// Detailed description of what the procedure involves
    pub description: String,
    /// Medical coding identifier (ICD-10-PCS, CPT, etc.)
    pub code: String,
    /// Category of procedure (e.g., "diagnostic", "surgical", "therapeutic")
    pub procedure_type: String,
}

impl Procedure {
    pub fn new(id: &str, name: &str, description: &str, code: &str, procedure_type: &str) -> Self {
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
pub struct Patient {
    /// Unique identifier for the patient (UUID)
    pub id: Uuid,
    /// Encrypted personally identifiable information
    pub pii: PatientPii,
}

impl Patient {
    pub fn new(id: &str, first_name: &str, last_name: &str, email: &str, date_of_birth: &str) -> Self {
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
pub struct PatientPii {
    /// Patient's first name
    pub first_name: String,
    /// Patient's last name
    pub last_name: String,
    /// Patient's email address for communication
    pub email: String,
    /// Patient's date of birth in ISO8601 format (YYYY-MM-DD)
    pub date_of_birth: String,
}

impl PatientPii {
    pub fn new(first_name: &str, last_name: &str, email: &str, date_of_birth: &str) -> Self {
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
pub struct EnhancedPatientPii {
    /// Patient's first name
    pub first_name: String,
    /// Patient's last name
    pub last_name: String,
    /// Patient's email address for communication
    pub email: String,
    /// Patient's date of birth in ISO8601 format (YYYY-MM-DD)
    pub date_of_birth: String,
    /// Complex medical history metadata
    pub medical_history: MedicalHistory,
    /// Insurance information
    pub insurance: InsuranceInfo,
    /// Current vital signs and measurements
    pub vitals: VitalSigns,
}

/// Medical history information containing arrays and nested data.
#[derive(Serialize)]
pub struct MedicalHistory {
    /// Known allergies as array of strings
    pub allergies: Vec<String>,
    /// Chronic conditions as array of strings
    pub conditions: Vec<String>,
    /// Emergency contact information
    pub emergency_contact: EmergencyContact,
    /// Risk factors with numeric scores
    pub risk_factors: RiskFactors,
}

/// Emergency contact details.
#[derive(Serialize)]
pub struct EmergencyContact {
    /// Contact person's name
    pub name: String,
    /// Contact phone number
    pub phone: String,
    /// Relationship to patient
    pub relationship: String,
}

/// Risk assessment scores.
#[derive(Serialize)]
pub struct RiskFactors {
    /// Cardiovascular risk score (0-100)
    pub cardiovascular: i32,
    /// Diabetes risk score (0-100)
    pub diabetes: i32,
    /// Overall health score (0-100)
    pub overall_health: i32,
}

/// Insurance provider information.
#[derive(Serialize)]
pub struct InsuranceInfo {
    /// Insurance provider name
    pub provider: String,
    /// Policy number
    pub policy_number: String,
    /// Group ID for employer plans
    pub group_id: i32,
    /// Coverage details
    pub coverage: CoverageDetails,
}

/// Insurance coverage breakdown.
#[derive(Serialize)]
pub struct CoverageDetails {
    /// Deductible amount in dollars
    pub deductible: i32,
    /// Out-of-pocket maximum
    pub out_of_pocket_max: i32,
    /// Copay amounts for different services
    pub copays: CopayInfo,
}

/// Copay information for different medical services.
#[derive(Serialize)]
pub struct CopayInfo {
    /// Primary care visit copay
    pub primary_care: i32,
    /// Specialist visit copay
    pub specialist: i32,
    /// Emergency room copay
    pub emergency: i32,
}

/// Current vital signs and physical measurements.
#[derive(Serialize)]
pub struct VitalSigns {
    /// Height in centimeters
    pub height_cm: i32,
    /// Weight in kilograms
    pub weight_kg: i32,
    /// Blood type (A+, A-, B+, B-, AB+, AB-, O+, O-)
    pub blood_type: String,
    /// Blood pressure readings
    pub blood_pressure: BloodPressure,
    /// Recent lab results
    pub lab_results: LabResults,
}

/// Blood pressure measurements.
#[derive(Serialize)]
pub struct BloodPressure {
    /// Systolic pressure
    pub systolic: i32,
    /// Diastolic pressure
    pub diastolic: i32,
    /// Date of measurement
    pub measured_date: String,
}

/// Laboratory test results.
#[derive(Serialize)]
pub struct LabResults {
    /// Cholesterol level (mg/dL)
    pub cholesterol: i32,
    /// Blood glucose level (mg/dL)
    pub glucose: i32,
    /// Hemoglobin A1C percentage
    pub hemoglobin_a1c: f32,
    /// Date of lab work
    pub test_date: String,
}

/// Enhanced patient with complex JSONB metadata.
#[derive(Serialize)]
pub struct EnhancedPatient {
    /// Unique identifier for the patient (UUID)
    pub id: Uuid,
    /// Enhanced PII with complex medical metadata
    pub pii: EnhancedPatientPii,
}

impl EnhancedPatient {
    pub fn new(
        id: &str,
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
pub struct Prescription {
    /// Reference to the patient receiving the prescription
    pub patient_id: Uuid,
    /// Encrypted prescription details
    pub prescription: PrescriptionDetail,
}

impl Prescription {
    pub fn new(
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
pub struct PrescriptionDetail {
    /// Reference to the prescribed medication
    pub medication_id: Uuid,
    /// Dosage instructions (e.g., "500mg twice daily", "as needed for pain")
    pub daily_dosage: String,
    /// Start date of the prescription in ISO8601 format
    pub from_date: String,
    /// End date of the prescription in ISO8601 format
    pub to_date: String,
}

impl PrescriptionDetail {
    pub fn new(medication_id: &str, daily_dosage: &str, from_date: &str, to_date: &str) -> Self {
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
pub struct PatientProcedure {
    /// Reference to the patient who received the procedure
    pub patient_id: Uuid,
    /// Encrypted procedure details
    pub procedure: ProcedureDetail,
}

impl PatientProcedure {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
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
pub struct ProcedureDetail {
    /// Reference to the type of procedure performed
    pub procedure_id: Uuid,
    /// Timestamp when the procedure was performed (ISO8601 format)
    pub when: String,
    /// Laterality of the procedure (e.g., "left", "right", "bilateral", "not applicable")
    pub laterality: String,
    /// Anatomical location where the procedure was performed
    pub body_site: String,
    /// Priority level of the procedure (e.g., "routine", "urgent", "elective")
    pub priority: String,
    /// Current status of the procedure (e.g., "completed", "in progress", "cancelled")
    pub status: String,
    /// Clinical diagnosis or condition before the procedure was performed
    pub preoperative_diagnosis: String,
    /// Clinical diagnosis or findings after the procedure was completed
    pub post_operative_diagnosis: String,
    /// Result or outcome of the procedure (e.g., "successful", "complicated", "ongoing")
    pub procedure_outcome: String,
}

impl ProcedureDetail {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
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
