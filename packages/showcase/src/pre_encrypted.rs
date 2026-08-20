//! Application-side encryption examples for Stash-style ingestion.
//!
//! Proxy accepts the resulting EQL storage payload as either a bound parameter
//! or a SQL literal, authenticates it, and avoids encrypting it a second time.

use crate::common::{connect_with_tls, PROXY};
use cipherstash_client::{
    encryption::{Plaintext, ScopedCipher},
    eql::{
        encrypt_eql_v3, EqlEncryptOpts, EqlOperation, EqlOutputV3, Identifier, PreparedPlaintext,
    },
    schema::{ColumnConfig, ColumnType},
    zerokms::{ClientKey, ZeroKMSBuilder},
    AutoStrategy, IdentifiedBy,
};
use cipherstash_config::column::{ArrayIndexMode, Index, IndexType, SteVecMode};
use serde_json::{json, Value};
use std::{borrow::Cow, sync::Arc};
use uuid::Uuid;

pub async fn run_examples() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔐 === Application-side EQL encryption ===");
    let client = connect_with_tls(*PROXY).await;

    // Example 1: bind an application-encrypted payload as a parameter.
    let parameter_id = Uuid::parse_str("a1b2c3d4-e5f6-4a5b-8c9d-123456789021")?;
    let parameter_pii = json!({
        "first_name": "Ada",
        "last_name": "Lovelace",
        "email": "ada@example.com",
        "date_of_birth": "1815-12-10"
    });
    let parameter_payload = encrypt_patient_pii(parameter_pii.clone()).await?;
    client
        .execute(
            "INSERT INTO patients (id, pii) VALUES ($1, $2)",
            &[&parameter_id, &parameter_payload],
        )
        .await?;
    println!("✅ Inserted application-encrypted PII as a bound parameter");

    // Example 2: the same wire payload can be supplied as a SQL literal.
    let literal_id = Uuid::parse_str("a1b2c3d4-e5f6-4a5b-8c9d-123456789022")?;
    let literal_pii = json!({
        "first_name": "Grace",
        "last_name": "Hopper",
        "email": "grace@example.com",
        "date_of_birth": "1906-12-09"
    });
    let literal_payload = encrypt_patient_pii(literal_pii.clone()).await?;
    let literal_payload = literal_payload.to_string().replace('\'', "''");
    client
        .simple_query(&format!(
            "INSERT INTO patients (id, pii) VALUES ('{literal_id}', '{literal_payload}')"
        ))
        .await?;
    println!("✅ Inserted application-encrypted PII as a SQL literal");

    // Both rows still decrypt normally when selected through Proxy.
    for (id, expected) in [(parameter_id, parameter_pii), (literal_id, literal_pii)] {
        let row = client
            .query_one("SELECT pii FROM patients WHERE id = $1", &[&id])
            .await?;
        assert_eq!(row.get::<_, Value>(0), expected);
    }
    println!("✅ Proxy authenticated and decrypted both application-encrypted values");
    Ok(())
}

async fn encrypt_patient_pii(value: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let config = ColumnConfig::build("patients/pii")
        .casts_as(ColumnType::Json)
        .add_index(Index::new(IndexType::SteVec {
            prefix: "patients/pii".into(),
            term_filters: Vec::new(),
            array_index_mode: ArrayIndexMode::ALL,
            mode: SteVecMode::default(),
        }));
    let prepared = PreparedPlaintext::new(
        Cow::Owned(config),
        Identifier::new("patients", "pii"),
        Plaintext::Json(Some(value)),
        EqlOperation::Store,
    );
    let mut outputs = encrypt_eql_v3(
        scoped_cipher().await?,
        vec![prepared],
        &EqlEncryptOpts::default(),
    )
    .await?;
    let EqlOutputV3::Store(ciphertext) = outputs.remove(0) else {
        return Err("store encryption returned a query payload".into());
    };
    Ok(serde_json::to_value(ciphertext)?)
}

async fn scoped_cipher() -> Result<Arc<ScopedCipher<AutoStrategy>>, Box<dyn std::error::Error>> {
    let client_id = env("CS_CLIENT_ID", "CS_ENCRYPT__CLIENT_ID")?.parse()?;
    let client_key =
        ClientKey::from_hex_v1(client_id, &env("CS_CLIENT_KEY", "CS_ENCRYPT__CLIENT_KEY")?)?;
    let zerokms = ZeroKMSBuilder::auto()?
        .with_client_key(client_key)
        .build()?;
    let keyset_id: Uuid = env("CS_DEFAULT_KEYSET_ID", "CS_ENCRYPT__DEFAULT_KEYSET_ID")?.parse()?;
    Ok(Arc::new(
        ScopedCipher::init(Arc::new(zerokms), Some(IdentifiedBy::Uuid(keyset_id))).await?,
    ))
}

fn env(primary: &str, nested: &str) -> Result<String, std::env::VarError> {
    std::env::var(primary).or_else(|_| std::env::var(nested))
}
