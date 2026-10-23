//! Application-side EQL examples for storage and search.
//!
//! Proxy accepts storage and query-only payloads as either bound parameters or
//! SQL literals, applies role-appropriate validation, and avoids encrypting
//! them a second time.

use crate::common::{connect_with_tls, PROXY};
use cipherstash_client::{
    encryption::{Plaintext, QueryOp, ScopedCipher},
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
    for (id, expected) in [
        (parameter_id, parameter_pii.clone()),
        (literal_id, literal_pii.clone()),
    ] {
        let row = client
            .query_one("SELECT pii FROM patients WHERE id = $1", &[&id])
            .await?;
        assert_eq!(row.get::<_, Value>(0), expected);
    }
    println!("✅ Proxy authenticated and decrypted both application-encrypted values");

    // Example 3: a query-only EQL payload contains SteVec SEM terms but no
    // source ciphertext. Proxy validates its query role and forwards it without
    // attempting authentication or encrypting it a second time.
    let parameter_query = query_patient_pii(parameter_pii).await?;
    let row = client
        .query_one(
            "SELECT id FROM patients WHERE pii @> $1",
            &[&parameter_query],
        )
        .await?;
    assert_eq!(row.get::<_, Uuid>(0), parameter_id);
    println!("✅ Queried with application-generated SEM terms as a bound parameter");

    // Example 4: query-only payloads are also accepted as SQL literals in
    // predicate positions.
    let literal_query = query_patient_pii(literal_pii)
        .await?
        .to_string()
        .replace('\'', "''");
    let rows = client
        .simple_query(&format!(
            "SELECT id FROM patients WHERE pii @> '{literal_query}'"
        ))
        .await?;
    let matched = rows.iter().any(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => {
            row.get(0) == Some(literal_id.to_string().as_str())
        }
        _ => false,
    });
    assert!(matched);
    println!("✅ Queried with application-generated SEM terms as a SQL literal");

    // Example 5: SteVec path selectors are bare, 32-character lowercase hex
    // query terms. Proxy recognises and forwards an application-generated hash
    // instead of hashing it again.
    let parameter_selector = query_patient_selector("$.first_name").await?;
    let row = client
        .query_one(
            "SELECT pii -> $1 FROM patients WHERE id = $2",
            &[&parameter_selector, &parameter_id],
        )
        .await?;
    assert_eq!(row.get::<_, Value>(0), json!("Ada"));
    println!("✅ Queried with an application-generated selector hash parameter");

    // Example 6: selector hashes work as literals too. A plaintext selector
    // matching the same format is ambiguous and is intentionally interpreted
    // as already hashed; see the showcase README for the compatibility rule.
    let literal_selector = query_patient_selector("$.first_name").await?;
    let rows = client
        .simple_query(&format!(
            "SELECT jsonb_path_query_first(pii, '{literal_selector}') \
             FROM patients WHERE id = '{literal_id}'"
        ))
        .await?;
    let selected = rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0),
        _ => None,
    });
    assert_eq!(selected, Some("\"Grace\""));
    println!("✅ Queried with an application-generated selector hash literal");
    Ok(())
}

async fn encrypt_patient_pii(value: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let config = patient_pii_config();
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

async fn query_patient_pii(value: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let config = patient_pii_config();
    let index_type = config.indexes[0].index_type.clone();
    let prepared = PreparedPlaintext::new(
        Cow::Owned(config),
        Identifier::new("patients", "pii"),
        Plaintext::Json(Some(value)),
        EqlOperation::Query(&index_type, QueryOp::Default),
    );
    let mut outputs = encrypt_eql_v3(
        scoped_cipher().await?,
        vec![prepared],
        &EqlEncryptOpts::default(),
    )
    .await?;
    let EqlOutputV3::Query(query) = outputs.remove(0) else {
        return Err("query encryption returned a storage payload".into());
    };
    Ok(serde_json::to_value(query)?)
}

async fn query_patient_selector(path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let config = patient_pii_config();
    let index_type = config.indexes[0].index_type.clone();
    let prepared = PreparedPlaintext::new(
        Cow::Owned(config),
        Identifier::new("patients", "pii"),
        Plaintext::from(path),
        EqlOperation::Query(&index_type, QueryOp::SteVecSelector),
    );
    let mut outputs = encrypt_eql_v3(
        scoped_cipher().await?,
        vec![prepared],
        &EqlEncryptOpts::default(),
    )
    .await?;
    let EqlOutputV3::Query(query) = outputs.remove(0) else {
        return Err("selector encryption returned a storage payload".into());
    };
    let Value::String(selector) = serde_json::to_value(query)? else {
        return Err("selector encryption returned a non-selector query payload".into());
    };
    Ok(selector)
}

fn patient_pii_config() -> ColumnConfig {
    ColumnConfig::build("patients/pii")
        .casts_as(ColumnType::Json)
        .add_index(Index::new(IndexType::SteVec {
            prefix: "patients/pii".into(),
            term_filters: Vec::new(),
            array_index_mode: ArrayIndexMode::ALL,
            mode: SteVecMode::default(),
        }))
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
