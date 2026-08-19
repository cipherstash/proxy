use crate::{error::EncryptError, postgresql::Column, EqlCiphertext};
use cipherstash_client::{
    eql::{EncryptedPayloadV3, EQL_SCHEMA_VERSION_V3},
    schema::column::IndexType,
};
use serde_json::Value;

/// Parse a value only when it advertises itself as an EQL storage payload.
/// Ordinary JSON remains plaintext; malformed payload-shaped JSON fails closed.
pub fn parse(bytes: &[u8], column: &Column) -> Result<Option<EqlCiphertext>, EncryptError> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };

    let payload_shaped = object.contains_key("c")
        || object.contains_key("h")
        || object.contains_key("sv") && object.contains_key("i");
    if !payload_shaped {
        return Ok(None);
    }

    let ciphertext: EqlCiphertext =
        serde_json::from_value(value).map_err(|_| EncryptError::InvalidInboundCiphertext)?;
    validate_metadata(&ciphertext, column)?;
    Ok(Some(ciphertext))
}

fn validate_metadata(ciphertext: &EqlCiphertext, column: &Column) -> Result<(), EncryptError> {
    if ciphertext.version() != EQL_SCHEMA_VERSION_V3
        || ciphertext.identifier() != &column.identifier
    {
        return Err(EncryptError::InvalidInboundCiphertext);
    }

    match ciphertext {
        EqlCiphertext::Encrypted(payload) => validate_scalar_terms(payload, column),
        EqlCiphertext::SteVec(payload) => {
            let configured = column
                .config
                .indexes
                .iter()
                .any(|index| matches!(index.index_type, IndexType::SteVec { .. }));
            if !configured || payload.ste_vec.is_empty() {
                return Err(EncryptError::InvalidInboundCiphertext);
            }
            Ok(())
        }
    }
}

fn validate_scalar_terms(
    payload: &EncryptedPayloadV3,
    column: &Column,
) -> Result<(), EncryptError> {
    let mut hmac = false;
    let mut bloom = false;
    let mut ore = false;
    let mut ope = false;
    for index in &column.config.indexes {
        match index.index_type {
            IndexType::Unique { .. } => hmac = true,
            IndexType::Match { .. } => bloom = true,
            IndexType::Ore => ore = true,
            IndexType::Ope => ope = true,
            IndexType::SteVec { .. } => return Err(EncryptError::InvalidInboundCiphertext),
        }
    }

    if payload.hmac_256.is_some() != hmac
        || payload.bloom_filter.is_some() != bloom
        || payload.ore_block_u64_8_256.is_some() != ore
        || payload.ope_cllw.is_some() != ope
    {
        return Err(EncryptError::InvalidInboundCiphertext);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cipherstash_client::schema::{ColumnConfig, ColumnMode, ColumnType};
    use cipherstash_client::zerokms::EncryptedRecord;
    use eql_mapper::EqlTermVariant;
    use uuid::Uuid;

    fn column() -> Column {
        Column {
            identifier: crate::Identifier::new("users", "email"),
            config: ColumnConfig {
                name: "email".into(),
                in_place: true,
                cast_type: ColumnType::Text,
                indexes: vec![],
                mode: ColumnMode::Encrypted,
            },
            postgres_type: postgres_types::Type::TEXT,
            eql_term: EqlTermVariant::Full,
        }
    }

    fn payload(identifier: crate::Identifier) -> EqlCiphertext {
        EqlCiphertext::Encrypted(EncryptedPayloadV3 {
            version: EQL_SCHEMA_VERSION_V3,
            identifier,
            ciphertext: EncryptedRecord {
                iv: Default::default(),
                ciphertext: vec![1; 16],
                tag: vec![2; 16],
                descriptor: "email".into(),
                keyset_id: Some(Uuid::nil()),
                decryption_policy: None,
            },
            hmac_256: None,
            bloom_filter: None,
            ore_block_u64_8_256: None,
            ope_cllw: None,
        })
    }

    #[test]
    fn ordinary_json_is_plaintext() {
        assert!(parse(br#"{"name":"Ada"}"#, &column()).unwrap().is_none());
    }

    #[test]
    fn malformed_payload_shape_fails_closed() {
        assert!(matches!(
            parse(br#"{"v":3,"i":"users.email","c":"bad"}"#, &column()),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }

    #[test]
    fn destination_identifier_must_match() {
        let ciphertext = payload(crate::Identifier::new("users", "phone"));
        assert!(matches!(
            validate_metadata(&ciphertext, &column()),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }

    #[test]
    fn configured_sem_terms_must_be_present() {
        let mut column = column();
        column
            .config
            .indexes
            .push(cipherstash_client::schema::column::Index::new_unique());
        let ciphertext = payload(column.identifier.clone());
        assert!(matches!(
            validate_metadata(&ciphertext, &column),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }
}
