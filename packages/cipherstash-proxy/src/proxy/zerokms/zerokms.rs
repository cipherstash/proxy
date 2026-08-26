use crate::{
    config::TandemConfig,
    error::{EncryptError, Error, ZeroKMSError},
    log::{ENCRYPT, ZEROKMS},
    postgresql::{Column, KeysetIdentifier},
    prometheus::{
        KEYSET_CIPHER_CACHE_HITS_TOTAL, KEYSET_CIPHER_CACHE_MISS_TOTAL,
        KEYSET_CIPHER_INIT_DURATION_SECONDS, KEYSET_CIPHER_INIT_TOTAL,
    },
    proxy::EncryptionService,
};
use cipherstash_client::{
    encryption::{DecryptOptions, Plaintext, QueryOp},
    eql::{
        encrypt_eql_v3, EqlCiphertextV3, EqlEncryptOpts, EqlOperation, EqlOutputV3,
        PreparedPlaintext, SteVecEntryV3,
    },
    schema::column::IndexType,
    zerokms::{Decryptable, EncryptedRecord, IdentifiedBy, RecordWithNonce, RetrieveKeyPayload},
};
use eql_mapper::EqlTermVariant;
use metrics::{counter, histogram};
use moka::future::Cache;
use std::convert::Infallible;
use std::{
    borrow::Cow,
    sync::Arc,
    time::{Duration, Instant},
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::{init_zerokms_client, ScopedCipher, ZerokmsClient};

/// Memory size of a single ScopedCipher instance for cache weighing
const SCOPED_CIPHER_SIZE: usize = std::mem::size_of::<ScopedCipher>();

/// An EQL v3 stored payload reduced to something the cipher can decrypt.
///
/// The two arms are not interchangeable, which is why this exists rather than a
/// plain `Vec<RecordWithNonce>`: `RecordWithNonce` unconditionally reports a
/// nonce override and an AAD selector, so wrapping a scalar record in one would
/// decrypt against a nonce the value was never encrypted with.
#[derive(Debug)]
enum V3Record {
    /// A scalar payload's `c` — self-describing, nonce derived from the data
    /// key's IV, nothing bound into the AAD.
    Scalar(EncryptedRecord),
    /// A SteVec entry, reassembled from the document's `h` header. Nonce and
    /// AAD both derive from the entry's selector.
    SteVecEntry(RecordWithNonce),
}

#[derive(Clone, Copy)]
enum SteVecAuthentication {
    RootOnly,
    AllEntries,
}

impl Decryptable for V3Record {
    type Error = Infallible;

    fn keyset_id(&self) -> Option<Uuid> {
        match self {
            V3Record::Scalar(record) => record.keyset_id(),
            V3Record::SteVecEntry(record) => record.keyset_id(),
        }
    }

    fn retrieve_key_payload(&self) -> Result<RetrieveKeyPayload<'_>, Self::Error> {
        match self {
            V3Record::Scalar(record) => record.retrieve_key_payload(),
            V3Record::SteVecEntry(record) => record.retrieve_key_payload(),
        }
    }

    fn into_encrypted_record(self) -> Result<EncryptedRecord, Self::Error> {
        match self {
            V3Record::Scalar(record) => record.into_encrypted_record(),
            V3Record::SteVecEntry(record) => record.into_encrypted_record(),
        }
    }

    fn nonce_override(&self) -> Option<[u8; 12]> {
        match self {
            V3Record::Scalar(_) => None,
            V3Record::SteVecEntry(record) => record.nonce_override(),
        }
    }

    fn aad_selector(&self) -> Option<[u8; 16]> {
        match self {
            V3Record::Scalar(_) => None,
            V3Record::SteVecEntry(record) => record.aad_selector(),
        }
    }
}

/// Decode a SteVec entry's hex-encoded tokenized selector into the 16 bytes the
/// AEAD binding needs.
fn decode_ste_vec_selector(selector: &str) -> Result<[u8; 16], EncryptError> {
    let bytes = hex::decode(selector).map_err(|_| EncryptError::SteVecSelectorInvalid {
        selector: selector.to_string(),
    })?;

    bytes
        .try_into()
        .map_err(|_| EncryptError::SteVecSelectorInvalid {
            selector: selector.to_string(),
        })
}

#[derive(Clone)]
pub struct ZeroKms {
    default_keyset_id: Option<Uuid>,
    zerokms_client: Arc<ZerokmsClient>,
    cipher_cache: Cache<String, Arc<ScopedCipher>>,
}

impl ZeroKms {
    pub fn init(config: &TandemConfig) -> Result<Self, Error> {
        let zerokms_client = init_zerokms_client(config)?;

        let cipher_cache = Cache::builder()
            // Use weigher to calculate actual memory usage of ScopedCipher instances
            .weigher(|_key: &String, _value: &Arc<ScopedCipher>| -> u32 {
                SCOPED_CIPHER_SIZE as u32
            })
            // Set capacity in bytes (entry count * actual struct size)
            .max_capacity((config.server.cipher_cache_size as u64) * SCOPED_CIPHER_SIZE as u64)
            .time_to_live(Duration::from_secs(config.server.cipher_cache_ttl_seconds))
            .eviction_listener(|key, _value, cause| {
                info!(target: ZEROKMS, msg = "ScopedCipher evicted from cache", cache_key = %key, cause = ?cause);
            })
            .build();

        let default_keyset_id = config.encrypt.default_keyset_id;

        Ok(ZeroKms {
            default_keyset_id,
            zerokms_client: Arc::new(zerokms_client),
            cipher_cache,
        })
    }

    /// Generate a cache key for the keyset identifier
    fn cache_key_for_keyset(keyset_id: &Option<KeysetIdentifier>) -> String {
        match keyset_id {
            Some(id) => format!("{}", id.0),
            None => "default".to_string(),
        }
    }

    /// Initialize cipher using the stored zerokms_config, with async Moka caching and memory tracking
    pub async fn init_cipher(
        &self,
        keyset_id: Option<KeysetIdentifier>,
    ) -> Result<Arc<ScopedCipher>, Error> {
        let cache_key = Self::cache_key_for_keyset(&keyset_id);

        // Check cache first
        if let Some(cached_cipher) = self.cipher_cache.get(&cache_key).await {
            debug!(target: ZEROKMS, msg = "Use cached ScopedCipher", ?keyset_id);
            counter!(KEYSET_CIPHER_CACHE_HITS_TOTAL).increment(1);
            return Ok(cached_cipher);
        }

        let zerokms_client = self.zerokms_client.clone();

        info!(target: ZEROKMS, msg = "Initializing ZeroKMS ScopedCipher (cache miss)", ?keyset_id);
        counter!(KEYSET_CIPHER_CACHE_MISS_TOTAL).increment(1);

        // A connection-level keyset takes precedence. Otherwise, scope the
        // cipher to Proxy's configured default instead of passing `None` and
        // silently falling back to the ZeroKMS client's account default. The
        // two defaults are not required to be the same, and using the account
        // default would derive different searchable-encryption terms.
        let identified_by = keyset_id
            .as_ref()
            .map(|id| id.0.clone())
            .or_else(|| self.default_keyset_id.map(IdentifiedBy::Uuid));

        let start = Instant::now();
        let result = ScopedCipher::init(zerokms_client, identified_by).await;
        let init_duration = start.elapsed();
        let init_duration_ms = init_duration.as_millis();

        if init_duration > Duration::from_secs(1) {
            warn!(target: ZEROKMS, msg = "Slow ScopedCipher initialization", ?keyset_id, init_duration_ms);
        }

        match result {
            Ok(cipher) => {
                let arc_cipher = Arc::new(cipher);

                counter!(KEYSET_CIPHER_INIT_TOTAL).increment(1);
                histogram!(KEYSET_CIPHER_INIT_DURATION_SECONDS).record(init_duration);

                // Store in cache
                self.cipher_cache
                    .insert(cache_key, arc_cipher.clone())
                    .await;

                // Update pending tasks to get accurate cache statistics
                self.cipher_cache.run_pending_tasks().await;

                let entry_count = self.cipher_cache.entry_count();
                let memory_usage_bytes = self.cipher_cache.weighted_size();

                info!(target: ZEROKMS, msg = "Connected to ZeroKMS", init_duration_ms);
                debug!(target: ZEROKMS, msg = "ScopedCipher cached", ?keyset_id, entry_count, memory_usage_bytes, init_duration_ms);

                Ok(arc_cipher)
            }
            Err(err) => {
                warn!(target: ZEROKMS, msg = "Error initializing ZeroKMS", error = err.to_string(), init_duration_ms);

                match err {
                    cipherstash_client::zerokms::Error::LoadKeyset(_) => {
                        Err(EncryptError::UnknownKeysetIdentifier {
                            keyset: keyset_id.map_or("default".to_string(), |id| id.to_string()),
                        }
                        .into())
                    }
                    cipherstash_client::zerokms::Error::Auth(_) => {
                        Err(ZeroKMSError::AuthenticationFailed.into())
                    }
                    _ => Err(Error::ZeroKMS(err.into())),
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl EncryptionService for ZeroKms {
    ///
    /// Encrypt `Plaintexts` using the `Column` configuration
    ///
    async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<EqlOutputV3>>, Error> {
        debug!(target: ENCRYPT, msg="Encrypt", ?keyset_id, default_keyset_id = ?self.default_keyset_id);

        // A keyset is required if no default keyset has been configured
        if self.default_keyset_id.is_none() && keyset_id.is_none() {
            return Err(EncryptError::MissingKeysetIdentifier.into());
        }

        let cipher = self.init_cipher(keyset_id.clone()).await?;

        // Collect indices and prepared plaintexts for non-None values
        let mut indices: Vec<usize> = Vec::new();
        let mut prepared_plaintexts: Vec<PreparedPlaintext> = Vec::new();

        for (idx, (plaintext_opt, col_opt)) in plaintexts.iter().zip(columns.iter()).enumerate() {
            if let (Some(plaintext), Some(col)) = (plaintext_opt, col_opt) {
                // Determine the EQL operation based on the term variant
                let eql_op = match col.eql_term {
                    // Full, Partial, and Tokenized terms store encrypted data with all indexes
                    EqlTermVariant::Full | EqlTermVariant::Partial | EqlTermVariant::Tokenized => {
                        EqlOperation::Store
                    }

                    // JsonPath generates a selector term for SteVec queries (e.g., jsonb_path_query)
                    EqlTermVariant::JsonPath => col
                        .config
                        .indexes
                        .iter()
                        .find(|i| matches!(i.index_type, IndexType::SteVec { .. }))
                        .map(|index| {
                            EqlOperation::Query(&index.index_type, QueryOp::SteVecSelector)
                        })
                        .unwrap_or(EqlOperation::Store),

                    // JsonAccessor generates a selector for SteVec field access (-> operator)
                    EqlTermVariant::JsonAccessor => col
                        .config
                        .indexes
                        .iter()
                        .find(|i| matches!(i.index_type, IndexType::SteVec { .. }))
                        .map(|index| {
                            EqlOperation::Query(&index.index_type, QueryOp::SteVecSelector)
                        })
                        .unwrap_or(EqlOperation::Store),

                    // JsonOrd is the scalar value operand of a JSON field ordering
                    // comparison (`col -> sel < value`): a SteVec ordering term
                    // (`{v,i,op}`) compared via `eql_v3.ord_term`.
                    EqlTermVariant::JsonOrd => col
                        .config
                        .indexes
                        .iter()
                        .find(|i| matches!(i.index_type, IndexType::SteVec { .. }))
                        .map(|index| EqlOperation::Query(&index.index_type, QueryOp::SteVecTerm))
                        .unwrap_or(EqlOperation::Store),

                    // JsonValueSelector is the fused value operand of a JSON
                    // field equality (`col -> sel = value`). Its plaintext is the
                    // composition input `{"path", "value"}` (built by the
                    // frontend from BOTH SQL operands); the client MACs them
                    // together into one selector, applying the column's term
                    // filters to the value. The result is a one-entry containment
                    // needle matched by `eql_v3.jsonb_contains`.
                    EqlTermVariant::JsonValueSelector => col
                        .config
                        .indexes
                        .iter()
                        .find(|i| matches!(i.index_type, IndexType::SteVec { .. }))
                        .map(|index| {
                            EqlOperation::Query(&index.index_type, QueryOp::SteVecValueSelector)
                        })
                        .unwrap_or(EqlOperation::Store),

                    // The result of an extraction, not an operand: it is read
                    // back from the database and decrypted, never encrypted on
                    // the way in. Refuse rather than fall through to `Store`,
                    // which would encrypt it in the wrong shape and silently
                    // return the wrong rows.
                    EqlTermVariant::JsonExtracted => {
                        return Err(EncryptError::JsonExtractedIsNotAnOperand.into())
                    }
                };

                let prepared = PreparedPlaintext::new(
                    Cow::Owned(col.config.clone()),
                    col.identifier.clone(),
                    plaintext.clone(),
                    eql_op,
                );
                indices.push(idx);
                prepared_plaintexts.push(prepared);
            }
        }

        // If no plaintexts to encrypt, return all None.
        //
        // Built by iteration rather than `vec![None; n]`: that needs `Clone`,
        // and `EqlOutputV3` does not derive it (neither does the v2
        // `EqlOutput` — the ciphertext types are `Clone`, the output wrappers
        // are not).
        if prepared_plaintexts.is_empty() {
            return Ok((0..plaintexts.len()).map(|_| None).collect());
        }

        // Use default opts since cipher is already initialized with the correct keyset
        let opts = EqlEncryptOpts::default();

        debug!(target: ENCRYPT, msg="Calling encrypt_eql_v3", count = prepared_plaintexts.len());
        let encrypt_start = Instant::now();
        let encrypted = encrypt_eql_v3(cipher, prepared_plaintexts, &opts)
            .await
            .map_err(EncryptError::from)?;
        let encrypt_duration = encrypt_start.elapsed();
        debug!(target: ENCRYPT, msg="encrypt_eql_v3 completed", count = encrypted.len(), duration_ms = encrypt_duration.as_millis());

        // Reconstruct the result vector with None values in the right places
        let mut result: Vec<Option<EqlOutputV3>> = (0..plaintexts.len()).map(|_| None).collect();
        for (idx, ciphertext) in indices.into_iter().zip(encrypted.into_iter()) {
            result[idx] = Some(ciphertext);
        }

        Ok(result)
    }

    ///
    /// Decrypt eql::Ciphertext into Plaintext
    ///
    /// Database values are stored as `eql::Ciphertext`
    ///
    async fn decrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<EqlCiphertextV3>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        self.decrypt_eql(keyset_id, ciphertexts, SteVecAuthentication::RootOnly)
            .await
    }

    async fn decrypt_inbound_eql(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<EqlCiphertextV3>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        self.decrypt_eql(keyset_id, ciphertexts, SteVecAuthentication::AllEntries)
            .await
    }
}

impl ZeroKms {
    async fn decrypt_eql(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<EqlCiphertextV3>>,
        ste_vec_authentication: SteVecAuthentication,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        debug!(target: ENCRYPT, msg="Decrypt", ?keyset_id, default_keyset_id = ?self.default_keyset_id);

        if self.default_keyset_id.is_none() && keyset_id.is_none() {
            return Err(EncryptError::MissingKeysetIdentifier.into());
        }

        let cipher = self.init_cipher(keyset_id).await?;
        if matches!(ste_vec_authentication, SteVecAuthentication::AllEntries)
            && ciphertexts
                .iter()
                .flatten()
                .any(|ciphertext| ciphertext_keyset_id(ciphertext) != Some(cipher.keyset_id()))
        {
            return Err(EncryptError::InvalidInboundEqlPayload.into());
        }

        // `decryption_policy` needs no parallel structural check here. For
        // tag-version 1 records ZeroKMS supplies and verifies the policy MAC
        // during key retrieval, so a forged policy fails authentication below.

        // Ordinary database reads authenticate only the root SteVec entry,
        // whose ciphertext contains the complete plaintext. Inbound storage
        // validation authenticates every entry because the whole application-
        // supplied document is about to become stored state.
        let mut result_positions: Vec<Option<usize>> = Vec::new();
        let mut records_to_decrypt: Vec<V3Record> = Vec::new();

        for (idx, ciphertext) in ciphertexts.iter().enumerate() {
            match ciphertext {
                Some(EqlCiphertextV3::Encrypted(payload)) => {
                    records_to_decrypt.push(V3Record::Scalar(payload.ciphertext.clone()));
                    result_positions.push(Some(idx));
                }
                Some(EqlCiphertextV3::SteVec(document)) => {
                    let entries =
                        ste_vec_entries_to_authenticate(&document.ste_vec, ste_vec_authentication)?;
                    for (entry_index, entry) in entries.iter().enumerate() {
                        let selector = decode_ste_vec_selector(&entry.selector)?;
                        records_to_decrypt.push(V3Record::SteVecEntry(
                            document
                                .key_header
                                .record_with_selector(entry.ciphertext.clone(), selector),
                        ));
                        result_positions.push((entry_index == 0).then_some(idx));
                    }
                }
                None => {}
            }
        }

        if records_to_decrypt.is_empty() {
            return Ok(vec![None; ciphertexts.len()]);
        }

        // The cipher is already scoped to the active keyset.
        let opts = DecryptOptions::default();
        debug!(target: ENCRYPT, msg="Decrypting EQL v3 records", count = records_to_decrypt.len());
        let decrypt_start = Instant::now();
        let decrypted = cipher
            .decrypt(records_to_decrypt, &opts)
            .await
            .map_err(ZeroKMSError::from)?;
        let decrypt_duration = decrypt_start.elapsed();
        debug!(target: ENCRYPT, msg="Decrypt completed", count = decrypted.len(), duration_ms = decrypt_duration.as_millis());

        // Non-root entries are authenticated but intentionally have no output
        // position: their decrypted sentinels are not legal Plaintext values.
        let mut result: Vec<Option<Plaintext>> = vec![None; ciphertexts.len()];
        for (result_position, bytes) in result_positions.into_iter().zip(decrypted) {
            if let Some(idx) = result_position {
                result[idx] = Some(Plaintext::from_slice(&bytes).map_err(EncryptError::from)?);
            }
        }

        Ok(result)
    }
}

fn ste_vec_entries_to_authenticate(
    entries: &[SteVecEntryV3],
    authentication: SteVecAuthentication,
) -> Result<&[SteVecEntryV3], EncryptError> {
    let root = entries
        .first()
        .ok_or(EncryptError::SteVecMissingRootEntry)?;
    Ok(match authentication {
        SteVecAuthentication::RootOnly => std::slice::from_ref(root),
        SteVecAuthentication::AllEntries => entries,
    })
}

fn ciphertext_keyset_id(ciphertext: &EqlCiphertextV3) -> Option<Uuid> {
    match ciphertext {
        EqlCiphertextV3::Encrypted(payload) => payload.ciphertext.keyset_id,
        EqlCiphertextV3::SteVec(document) => document.key_header.keyset_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ste_vec_entry(selector: &str) -> SteVecEntryV3 {
        SteVecEntryV3 {
            selector: selector.into(),
            ciphertext: vec![1; 16],
            is_array: None,
            term: None,
        }
    }

    #[test]
    fn ordinary_decryption_authenticates_only_the_ste_vec_root() {
        let entries = vec![ste_vec_entry("root"), ste_vec_entry("nested")];

        assert_eq!(
            ste_vec_entries_to_authenticate(&entries, SteVecAuthentication::RootOnly)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn inbound_validation_authenticates_every_ste_vec_entry() {
        let entries = vec![ste_vec_entry("root"), ste_vec_entry("nested")];

        assert_eq!(
            ste_vec_entries_to_authenticate(&entries, SteVecAuthentication::AllEntries)
                .unwrap()
                .len(),
            entries.len()
        );
    }
}
