use crate::{error::EncryptError, postgresql::Column, EqlCiphertext, EqlQueryPayload};
use cipherstash_client::{
    eql::{EncryptedPayloadV3, EQL_SCHEMA_VERSION_V3},
    schema::column::IndexType,
};
use eql_mapper::EqlTermVariant;
use serde_json::Value;

/// An application-generated EQL value entering Proxy.
#[derive(Debug)]
pub enum InboundEql {
    /// A stored payload carrying source ciphertext. This must be authenticated
    /// and have its SEM terms independently verified before it can be used.
    Store(EqlCiphertext),
    /// A query operand carrying SEM terms only. It can never be written and has
    /// no source ciphertext with which to authenticate its metadata.
    Query(EqlQueryPayload),
}

/// Parse a value only when its fields advertise it as an EQL storage payload or
/// query operand. Query-only payloads are valid exclusively in syntactic query
/// positions. Ordinary JSON (including an object with a `c` key) remains
/// plaintext; malformed advertised payloads fail closed.
pub fn parse(
    bytes: &[u8],
    column: &Column,
    query_operand: bool,
) -> Result<Option<InboundEql>, EncryptError> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };

    let storage_shaped = object.contains_key("v")
        && object.contains_key("i")
        && (object.contains_key("c") || object.contains_key("h") || object.contains_key("sv"));
    if storage_shaped {
        let ciphertext: EqlCiphertext =
            serde_json::from_value(value).map_err(|_| EncryptError::InvalidInboundCiphertext)?;
        validate_storage_metadata(&ciphertext, column)?;
        return Ok(Some(InboundEql::Store(ciphertext)));
    }

    let scalar_query_shaped = object.contains_key("v")
        && object.contains_key("i")
        && ["hm", "bf", "ob", "op"]
            .iter()
            .any(|term| object.contains_key(*term));
    let ste_vec_query_shaped = object.len() == 1 && object.contains_key("sv");
    if !scalar_query_shaped && !ste_vec_query_shaped {
        return Ok(None);
    }
    if !query_operand {
        return Err(EncryptError::InvalidInboundCiphertext);
    }

    let query: EqlQueryPayload =
        serde_json::from_value(value).map_err(|_| EncryptError::InvalidInboundCiphertext)?;
    validate_query_metadata(&query, column)?;
    Ok(Some(InboundEql::Query(query)))
}

fn validate_storage_metadata(
    ciphertext: &EqlCiphertext,
    column: &Column,
) -> Result<(), EncryptError> {
    if ciphertext.version() != EQL_SCHEMA_VERSION_V3
        || ciphertext.identifier() != &column.identifier
    {
        return Err(EncryptError::InvalidInboundCiphertext);
    }

    // The descriptor is covered by the encrypted record's AEAD tag. Requiring
    // the canonical table/column descriptor cryptographically binds a payload
    // to its claimed destination, unlike the self-reported `i` field alone.
    let expected_descriptor = format!("{}/{}", column.identifier.table, column.identifier.column);
    let descriptor = match ciphertext {
        EqlCiphertext::Encrypted(payload) => &payload.ciphertext.descriptor,
        EqlCiphertext::SteVec(payload) => &payload.key_header.descriptor,
    };
    if descriptor != &expected_descriptor {
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

fn validate_query_metadata(query: &EqlQueryPayload, column: &Column) -> Result<(), EncryptError> {
    match query {
        EqlQueryPayload::Encrypted(payload) => {
            if payload.version != EQL_SCHEMA_VERSION_V3 || payload.identifier != column.identifier {
                return Err(EncryptError::InvalidInboundCiphertext);
            }

            match column.eql_term {
                EqlTermVariant::Full | EqlTermVariant::Partial | EqlTermVariant::Tokenized => {
                    validate_scalar_term_presence(
                        payload.hmac_256.is_some(),
                        payload.bloom_filter.is_some(),
                        payload.ore_block_u64_8_256.is_some(),
                        payload.ope_cllw.is_some(),
                        column,
                    )
                }
                EqlTermVariant::JsonOrd => {
                    let ste_vec_configured = column
                        .config
                        .indexes
                        .iter()
                        .any(|index| matches!(index.index_type, IndexType::SteVec { .. }));
                    if !ste_vec_configured
                        || payload.hmac_256.is_some()
                        || payload.bloom_filter.is_some()
                        || payload.ore_block_u64_8_256.is_some()
                        || payload.ope_cllw.is_none()
                    {
                        return Err(EncryptError::InvalidInboundCiphertext);
                    }
                    Ok(())
                }
                _ => Err(EncryptError::InvalidInboundCiphertext),
            }
        }
        EqlQueryPayload::SteVec(payload) => {
            let configured = column
                .config
                .indexes
                .iter()
                .any(|index| matches!(index.index_type, IndexType::SteVec { .. }));
            let query_shape = matches!(
                column.eql_term,
                EqlTermVariant::Full | EqlTermVariant::Partial | EqlTermVariant::JsonValueSelector
            );
            if !configured || !query_shape || payload.ste_vec.is_empty() {
                return Err(EncryptError::InvalidInboundCiphertext);
            }
            Ok(())
        }
        // Bare selector hashes are indistinguishable from ordinary plaintext
        // text on the PostgreSQL wire, so they cannot safely advertise
        // themselves as pre-computed query operands.
        EqlQueryPayload::Selector(_) => Err(EncryptError::InvalidInboundCiphertext),
    }
}

/// Compare all searchable metadata after the plaintext has been authenticated
/// and independently re-encrypted for the inferred destination column.
/// `into_query_operand` removes only record ciphertext/key material, leaving
/// the identifier and every scalar or SteVec SEM term. Bloom-filter positions
/// are compared without regard to order; all other terms compare exactly.
pub fn sem_terms_match(inbound: &EqlCiphertext, derived: EqlCiphertext) -> bool {
    if let (EqlCiphertext::Encrypted(inbound), EqlCiphertext::Encrypted(derived)) =
        (inbound, &derived)
    {
        return inbound.version == derived.version
            && inbound.identifier == derived.identifier
            && inbound.hmac_256 == derived.hmac_256
            && bloom_filters_match(&inbound.bloom_filter, &derived.bloom_filter)
            && inbound.ore_block_u64_8_256 == derived.ore_block_u64_8_256
            && inbound.ope_cllw == derived.ope_cllw;
    }

    match (
        serde_json::to_value(inbound.clone().into_query_operand()),
        serde_json::to_value(derived.into_query_operand()),
    ) {
        (Ok(inbound), Ok(derived)) => inbound == derived,
        _ => false,
    }
}

fn bloom_filters_match(inbound: &Option<Vec<i16>>, derived: &Option<Vec<i16>>) -> bool {
    match (inbound, derived) {
        (Some(inbound), Some(derived)) => {
            // Bloom-filter positions are a set. Their generation order is not
            // stable, so comparing the serialized arrays directly rejects
            // equivalent terms produced by independent encryptions.
            let mut inbound = inbound.clone();
            let mut derived = derived.clone();
            inbound.sort_unstable();
            derived.sort_unstable();
            inbound == derived
        }
        (None, None) => true,
        _ => false,
    }
}

fn validate_scalar_terms(
    payload: &EncryptedPayloadV3,
    column: &Column,
) -> Result<(), EncryptError> {
    validate_scalar_term_presence(
        payload.hmac_256.is_some(),
        payload.bloom_filter.is_some(),
        payload.ore_block_u64_8_256.is_some(),
        payload.ope_cllw.is_some(),
        column,
    )
}

fn validate_scalar_term_presence(
    has_hmac: bool,
    has_bloom: bool,
    has_ore: bool,
    has_ope: bool,
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

    if has_hmac != hmac || has_bloom != bloom || has_ore != ore || has_ope != ope {
        return Err(EncryptError::InvalidInboundCiphertext);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cipherstash_client::schema::{ColumnConfig, ColumnMode, ColumnType};
    use cipherstash_client::zerokms::EncryptedRecord;
    use cipherstash_config::column::{ArrayIndexMode, Index, SteVecMode};
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
                descriptor: "users/email".into(),
                keyset_id: Some(Uuid::nil()),
                decryption_policy: None,
            },
            hmac_256: None,
            bloom_filter: None,
            ore_block_u64_8_256: None,
            ope_cllw: None,
        })
    }

    fn ste_vec_column() -> Column {
        let mut column = column();
        column.config.cast_type = ColumnType::Json;
        column.config.indexes.push(Index::new(IndexType::SteVec {
            prefix: "users/email".into(),
            term_filters: Vec::new(),
            array_index_mode: ArrayIndexMode::ALL,
            mode: SteVecMode::default(),
        }));
        column.postgres_type = postgres_types::Type::JSONB;
        column
    }

    #[test]
    fn ordinary_json_is_plaintext() {
        assert!(parse(br#"{"name":"Ada"}"#, &column(), false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn ordinary_json_with_a_c_key_is_plaintext() {
        assert!(parse(br#"{"c":"customer code"}"#, &column(), false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn malformed_payload_shape_fails_closed() {
        assert!(matches!(
            parse(br#"{"v":3,"i":"users.email","c":"bad"}"#, &column(), false),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }

    #[test]
    fn destination_identifier_must_match() {
        let ciphertext = payload(crate::Identifier::new("users", "phone"));
        assert!(matches!(
            validate_storage_metadata(&ciphertext, &column()),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }

    #[test]
    fn authenticated_descriptor_must_match_destination() {
        let mut ciphertext = payload(crate::Identifier::new("users", "email"));
        let EqlCiphertext::Encrypted(payload) = &mut ciphertext else {
            unreachable!()
        };
        payload.ciphertext.descriptor = "accounts/email".into();
        assert!(matches!(
            validate_storage_metadata(&ciphertext, &column()),
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
            validate_storage_metadata(&ciphertext, &column),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }

    #[test]
    fn independently_derived_sem_terms_must_match() {
        let derived = payload(crate::Identifier::new("users", "email"));
        let mut spliced = derived.clone();
        let EqlCiphertext::Encrypted(payload) = &mut spliced else {
            unreachable!()
        };
        payload.hmac_256 = Some("term from another plaintext".into());

        assert!(!sem_terms_match(&spliced, derived));
    }

    #[test]
    fn bloom_filter_order_does_not_affect_sem_term_matching() {
        let mut inbound = payload(crate::Identifier::new("users", "email"));
        let mut derived = inbound.clone();
        if let EqlCiphertext::Encrypted(payload) = &mut inbound {
            payload.bloom_filter = Some(vec![3, 1, 2]);
        }
        if let EqlCiphertext::Encrypted(payload) = &mut derived {
            payload.bloom_filter = Some(vec![1, 2, 3]);
        }

        assert!(sem_terms_match(&inbound, derived));
    }

    #[test]
    fn query_only_scalar_payload_is_accepted_for_a_query_operand() {
        let mut column = column();
        column
            .config
            .indexes
            .push(cipherstash_client::schema::column::Index::new_unique());
        let mut ciphertext = payload(column.identifier.clone());
        let EqlCiphertext::Encrypted(payload) = &mut ciphertext else {
            unreachable!()
        };
        payload.hmac_256 = Some("application-generated SEM term".into());
        let query = serde_json::to_vec(&ciphertext.into_query_operand()).unwrap();

        assert!(matches!(
            parse(&query, &column, true),
            Ok(Some(InboundEql::Query(_)))
        ));
    }

    #[test]
    fn query_only_scalar_payload_is_rejected_for_storage() {
        let mut column = column();
        column
            .config
            .indexes
            .push(cipherstash_client::schema::column::Index::new_unique());
        let mut ciphertext = payload(column.identifier.clone());
        let EqlCiphertext::Encrypted(payload) = &mut ciphertext else {
            unreachable!()
        };
        payload.hmac_256 = Some("application-generated SEM term".into());
        let query = serde_json::to_vec(&ciphertext.into_query_operand()).unwrap();

        assert!(matches!(
            parse(&query, &column, false),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }

    #[test]
    fn query_only_scalar_identifier_must_match_destination() {
        let mut column = column();
        column
            .config
            .indexes
            .push(cipherstash_client::schema::column::Index::new_unique());
        let mut ciphertext = payload(crate::Identifier::new("users", "phone"));
        let EqlCiphertext::Encrypted(payload) = &mut ciphertext else {
            unreachable!()
        };
        payload.hmac_256 = Some("application-generated SEM term".into());
        let query = serde_json::to_vec(&ciphertext.into_query_operand()).unwrap();

        assert!(matches!(
            parse(&query, &column, true),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }

    #[test]
    fn query_only_json_ordering_term_is_accepted() {
        let mut column = ste_vec_column();
        column.eql_term = EqlTermVariant::JsonOrd;
        let query = serde_json::to_vec(&serde_json::json!({
            "v": EQL_SCHEMA_VERSION_V3,
            "i": { "t": "users", "c": "email" },
            "op": "application-generated ordering term"
        }))
        .unwrap();

        assert!(matches!(
            parse(&query, &column, true),
            Ok(Some(InboundEql::Query(EqlQueryPayload::Encrypted(_))))
        ));
    }

    #[test]
    fn query_only_ste_vec_payload_is_accepted_for_a_query_operand() {
        let query = br#"{"sv":[{"s":"application-generated selector"}]}"#;

        assert!(matches!(
            parse(query, &ste_vec_column(), true),
            Ok(Some(InboundEql::Query(EqlQueryPayload::SteVec(_))))
        ));
    }

    #[test]
    fn query_only_ste_vec_payload_is_rejected_for_storage() {
        let query = br#"{"sv":[{"s":"application-generated selector"}]}"#;

        assert!(matches!(
            parse(query, &ste_vec_column(), false),
            Err(EncryptError::InvalidInboundCiphertext)
        ));
    }
}
