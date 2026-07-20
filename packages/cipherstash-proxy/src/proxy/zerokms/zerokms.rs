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
        PreparedPlaintext,
    },
    schema::column::IndexType,
<<<<<<< HEAD
<<<<<<< HEAD
    zerokms::{Decryptable, EncryptedRecord, RecordWithNonce, RetrieveKeyPayload},
=======
    zerokms::WithContext,
>>>>>>> 905dfb04 (feat(encrypt): produce EQL v3 payloads, retire v2)
=======
    zerokms::{Decryptable, EncryptedRecord, RecordWithNonce, RetrieveKeyPayload},
>>>>>>> 2291ca06 (fix(decrypt): implement EQL v3 decrypt for scalar and SteVec payloads)
};
use std::convert::Infallible;
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
    /// A SteVec document's root entry, reassembled from the document's `h`
    /// header. Nonce and AAD both derive from the entry's selector.
    SteVecRoot(RecordWithNonce),
}

impl Decryptable for V3Record {
    type Error = Infallible;

    fn keyset_id(&self) -> Option<Uuid> {
        match self {
            V3Record::Scalar(record) => record.keyset_id(),
            V3Record::SteVecRoot(record) => record.keyset_id(),
        }
    }

    fn retrieve_key_payload(&self) -> Result<RetrieveKeyPayload<'_>, Self::Error> {
        match self {
            V3Record::Scalar(record) => record.retrieve_key_payload(),
            V3Record::SteVecRoot(record) => record.retrieve_key_payload(),
        }
    }

    fn into_encrypted_record(self) -> Result<EncryptedRecord, Self::Error> {
        match self {
            V3Record::Scalar(record) => record.into_encrypted_record(),
            V3Record::SteVecRoot(record) => record.into_encrypted_record(),
        }
    }

    fn nonce_override(&self) -> Option<[u8; 12]> {
        match self {
            V3Record::Scalar(_) => None,
            V3Record::SteVecRoot(record) => record.nonce_override(),
        }
    }

    fn aad_selector(&self) -> Option<[u8; 16]> {
        match self {
            V3Record::Scalar(_) => None,
            V3Record::SteVecRoot(record) => record.aad_selector(),
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

        let identified_by = keyset_id.as_ref().map(|id| id.0.clone());

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
<<<<<<< HEAD
        let mut result: Vec<Option<EqlOutputV3>> = (0..plaintexts.len()).map(|_| None).collect();
=======
        let mut result: Vec<Option<EqlOutputV3>> = vec![None; plaintexts.len()];
>>>>>>> 905dfb04 (feat(encrypt): produce EQL v3 payloads, retire v2)
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
        debug!(target: ENCRYPT, msg="Decrypt", ?keyset_id, default_keyset_id = ?self.default_keyset_id);

        // A keyset is required if no default keyset has been configured
        if self.default_keyset_id.is_none() && keyset_id.is_none() {
            return Err(EncryptError::MissingKeysetIdentifier.into());
        }

        let cipher = self.init_cipher(keyset_id.clone()).await?;

        // Collect indices and the root records for non-None values.
        //
        // cipherstash-client has no `decrypt_eql_v3` counterpart to
        // `encrypt_eql_v3` — the v2 `decrypt_eql` only accepts `EqlCiphertext`.
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> 2291ca06 (fix(decrypt): implement EQL v3 decrypt for scalar and SteVec payloads)
        // We assemble the decryptable record ourselves, which is what
        // protect-ffi does too (`encrypted_record_from_value`).
        //
        // Scalar: `c` is already the `EncryptedRecord` the v2 path would have
        // unwrapped, and `EncryptedRecord` is `Decryptable`.
        //
        // SteVec: the document holds the key material once in the `h` header
        // and each entry carries only raw AEAD bytes, so the record has to be
        // reassembled from the header plus the ROOT entry (`sv[0]`, the same
        // decryption-root invariant v2 had). The selector is the AEAD binding —
        // its first 12 bytes are the nonce and all 16 go into the AAD — which is
        // why the reassembled record is a `RecordWithNonce`.
<<<<<<< HEAD
        let mut indices: Vec<usize> = Vec::new();
        let mut records_to_decrypt: Vec<V3Record> = Vec::new();
=======
        // For scalar payloads that costs us nothing: `EncryptedPayloadV3.c` is
        // the same `EncryptedRecord` the v2 path would have unwrapped, and
        // `EncryptedRecord` is itself `Decryptable`, so we hand it straight to
        // the cipher. SteVec documents are the part that genuinely needs the
        // client (see `SteVecV3DecryptUnsupported`).
        let mut indices: Vec<usize> = Vec::new();
        let mut records_to_decrypt = Vec::new();
>>>>>>> 905dfb04 (feat(encrypt): produce EQL v3 payloads, retire v2)
=======
        let mut indices: Vec<usize> = Vec::new();
        let mut records_to_decrypt: Vec<V3Record> = Vec::new();
>>>>>>> 2291ca06 (fix(decrypt): implement EQL v3 decrypt for scalar and SteVec payloads)

        for (idx, ct_opt) in ciphertexts.iter().enumerate() {
            if let Some(ct) = ct_opt {
                let record = match ct {
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> 2291ca06 (fix(decrypt): implement EQL v3 decrypt for scalar and SteVec payloads)
                    EqlCiphertextV3::Encrypted(payload) => {
                        V3Record::Scalar(payload.ciphertext.clone())
                    }
                    EqlCiphertextV3::SteVec(document) => {
                        let root = document
                            .ste_vec
                            .first()
                            .ok_or(EncryptError::SteVecMissingRootEntry)?;

                        let selector = decode_ste_vec_selector(&root.selector)?;
                        V3Record::SteVecRoot(
                            document
                                .key_header
                                .record_with_selector(root.ciphertext.clone(), selector),
                        )
<<<<<<< HEAD
=======
                    EqlCiphertextV3::Encrypted(payload) => payload.ciphertext.clone(),
                    EqlCiphertextV3::SteVec(_) => {
                        return Err(EncryptError::SteVecV3DecryptUnsupported.into())
>>>>>>> 905dfb04 (feat(encrypt): produce EQL v3 payloads, retire v2)
=======
>>>>>>> 2291ca06 (fix(decrypt): implement EQL v3 decrypt for scalar and SteVec payloads)
                    }
                };
                indices.push(idx);
                records_to_decrypt.push(record);
            }
        }

        // If no ciphertexts to decrypt, return all None
        if records_to_decrypt.is_empty() {
            return Ok(vec![None; ciphertexts.len()]);
        }

        // Default opts: the cipher is already scoped to the right keyset, and
        // Proxy does not set a lock context.
        let opts = DecryptOptions::default();

        debug!(target: ENCRYPT, msg="Decrypting EQL v3 records", count = records_to_decrypt.len());
        let decrypt_start = Instant::now();
        let decrypted = cipher
            .decrypt(records_to_decrypt, &opts)
            .await
            .map_err(ZeroKMSError::from)?
            .into_iter()
            .map(|bytes| Plaintext::from_slice(&bytes))
            .collect::<Result<Vec<_>, _>>()
            .map_err(EncryptError::from)?;
        let decrypt_duration = decrypt_start.elapsed();
        debug!(target: ENCRYPT, msg="Decrypt completed", count = decrypted.len(), duration_ms = decrypt_duration.as_millis());

        // Reconstruct the result vector with None values in the right places
        let mut result: Vec<Option<Plaintext>> = vec![None; ciphertexts.len()];
        for (idx, plaintext) in indices.into_iter().zip(decrypted.into_iter()) {
            result[idx] = Some(plaintext);
        }

        Ok(result)
    }
}
