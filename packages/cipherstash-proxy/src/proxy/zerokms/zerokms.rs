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
    encryption::{Plaintext, QueryOp},
    eql::{
        decrypt_eql, encrypt_eql, EqlCiphertext, EqlDecryptOpts, EqlEncryptOpts, EqlOperation,
        PreparedPlaintext,
    },
    schema::column::IndexType,
};
use eql_mapper::EqlTermVariant;
use metrics::{counter, histogram};
use moka::future::Cache;
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
                    cipherstash_client::zerokms::Error::Credentials(_) => {
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
    ) -> Result<Vec<Option<EqlCiphertext>>, Error> {
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

        // If no plaintexts to encrypt, return all None
        if prepared_plaintexts.is_empty() {
            return Ok(vec![None; plaintexts.len()]);
        }

        // Use default opts since cipher is already initialized with the correct keyset
        let opts = EqlEncryptOpts::default();

        debug!(target: ENCRYPT, msg="Calling encrypt_eql", count = prepared_plaintexts.len());
        let encrypt_start = Instant::now();
        let encrypted = encrypt_eql(cipher, prepared_plaintexts, &opts)
            .await
            .map_err(EncryptError::from)?;
        let encrypt_duration = encrypt_start.elapsed();
        debug!(target: ENCRYPT, msg="encrypt_eql completed", count = encrypted.len(), duration_ms = encrypt_duration.as_millis());

        // Reconstruct the result vector with None values in the right places
        let mut result: Vec<Option<EqlCiphertext>> = vec![None; plaintexts.len()];
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
        ciphertexts: Vec<Option<EqlCiphertext>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        debug!(target: ENCRYPT, msg="Decrypt", ?keyset_id, default_keyset_id = ?self.default_keyset_id);

        // A keyset is required if no default keyset has been configured
        if self.default_keyset_id.is_none() && keyset_id.is_none() {
            return Err(EncryptError::MissingKeysetIdentifier.into());
        }

        let cipher = self.init_cipher(keyset_id.clone()).await?;

        // Collect indices and ciphertexts for non-None values
        let mut indices: Vec<usize> = Vec::new();
        let mut ciphertexts_to_decrypt: Vec<EqlCiphertext> = Vec::new();

        for (idx, ct_opt) in ciphertexts.iter().enumerate() {
            if let Some(ct) = ct_opt {
                indices.push(idx);
                ciphertexts_to_decrypt.push(ct.clone());
            }
        }

        // If no ciphertexts to decrypt, return all None
        if ciphertexts_to_decrypt.is_empty() {
            return Ok(vec![None; ciphertexts.len()]);
        }

        // Use default opts since cipher is already initialized with the correct keyset
        let opts = EqlDecryptOpts::default();

        debug!(target: ENCRYPT, msg="Calling decrypt_eql", count = ciphertexts_to_decrypt.len());
        let decrypt_start = Instant::now();
        let decrypted = decrypt_eql(cipher, ciphertexts_to_decrypt, &opts)
            .await
            .map_err(EncryptError::from)?;
        let decrypt_duration = decrypt_start.elapsed();
        debug!(target: ENCRYPT, msg="decrypt_eql completed", count = decrypted.len(), duration_ms = decrypt_duration.as_millis());

        // Reconstruct the result vector with None values in the right places
        let mut result: Vec<Option<Plaintext>> = vec![None; ciphertexts.len()];
        for (idx, plaintext) in indices.into_iter().zip(decrypted.into_iter()) {
            result[idx] = Some(plaintext);
        }

        Ok(result)
    }
}
