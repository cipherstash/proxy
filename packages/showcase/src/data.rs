use crate::{
    common::{connect_with_tls, insert, table_exists, PROXY},
    model::{
        BloodPressure, CopayInfo, CoverageDetails, EmergencyContact, InsuranceInfo, LabResults,
        MedicalHistory, Medication, Patient, PatientProcedure, Prescription, Procedure,
        RiskFactors, VitalSigns,
    },
};

pub async fn insert_test_data() {
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

pub async fn clear() {
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

/// Creates enhanced JSONB test data with complex nested medical information.
pub async fn create_enhanced_jsonb_test_data() {
    println!("📋 Creating enhanced JSONB test data...");

    let enhanced_patients = [
        // Patient 1: John Smith with complex medical data
        Patient::new_enhanced(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789011",
            "John",
            "Smith",
            "john.smith@email.com",
            "1985-03-15",
            MedicalHistory {
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
            InsuranceInfo {
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
            VitalSigns {
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
        ),
        // Patient 2: Sarah Johnson with different insurance
        Patient::new_enhanced(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789012",
            "Sarah",
            "Johnson",
            "sarah.johnson@gmail.com",
            "1992-07-28",
            MedicalHistory {
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
            InsuranceInfo {
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
            VitalSigns {
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
        ),
        // Patient 3: Michael Brown with high risk factors
        Patient::new_enhanced(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789013",
            "Michael",
            "Brown",
            "m.brown@outlook.com",
            "1978-12-03",
            MedicalHistory {
                allergies: vec![
                    "latex".to_string(),
                    "sulfa".to_string(),
                    "iodine".to_string(),
                ],
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
            InsuranceInfo {
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
            VitalSigns {
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
        ),
    ];

    for patient in &enhanced_patients {
        let pii_json = serde_json::to_value(&patient.pii).unwrap();
        let sql = "INSERT INTO patients (id, pii) VALUES ($1, $2)";
        insert(sql, &[&patient.id, &pii_json]).await;
    }

    // Add prescriptions for enhanced patients so they appear in complex queries
    let enhanced_prescriptions = [
        // Patient 1 (John Smith - HealthCorp insurance) - active prescriptions
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789011",
            "550e8400-e29b-41d4-a716-446655440005",
            "500mg twice daily",
            "2024-01-20",
            "2024-12-31",
        ), // Metformin
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789011",
            "550e8400-e29b-41d4-a716-446655440006",
            "10mg once daily",
            "2024-02-01",
            "2024-12-31",
        ), // Lisinopril
        // Patient 2 (Sarah Johnson - BlueCross insurance) - active prescriptions
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789012",
            "550e8400-e29b-41d4-a716-446655440002",
            "200mg as needed",
            "2024-01-25",
            "2024-08-31",
        ), // Ibuprofen
        // Patient 3 (Michael Brown - HealthCorp insurance) - active prescriptions
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789013",
            "550e8400-e29b-41d4-a716-446655440007",
            "40mg once daily",
            "2024-02-15",
            "2024-12-31",
        ), // Atorvastatin
        Prescription::new(
            "a1b2c3d4-e5f6-4a5b-8c9d-123456789013",
            "550e8400-e29b-41d4-a716-446655440009",
            "50mg once daily",
            "2024-03-01",
            "2024-12-31",
        ), // Losartan
    ];

    for prescription in &enhanced_prescriptions {
        let prescription_json = serde_json::to_value(&prescription.prescription).unwrap();
        let sql = "INSERT INTO patient_medications (patient_id, medication) VALUES ($1, $2)";
        insert(sql, &[&prescription.patient_id, &prescription_json]).await;
    }

    println!("✅ Enhanced JSONB test data created successfully");
}
