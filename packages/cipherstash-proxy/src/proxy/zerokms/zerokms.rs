use crate::{
    config::TandemConfig,
    eql,
    error::{EncryptError, Error},
    log::ENCRYPT,
    postgresql::{Column, KeysetIdentifier},
    prometheus::{KEYSET_CIPHER_CACHE_HITS_TOTAL, KEYSET_CIPHER_INIT_TOTAL},
};
use cipherstash_client::{
    encryption::QueryOp,
    encryption::{Plaintext, PlaintextTarget, Queryable, ReferencedPendingPipeline},
};
use metrics::counter;
use moka::future::Cache;
use std::{sync::Arc, time::Duration};
use tracing::debug;

use super::{
    init_zerokms_client, plaintext_type_name, to_eql_encrypted, to_eql_encrypted_from_index_term,
    ScopedCipher, ZerokmsClient,
};

/// Memory size of a single ScopedCipher instance for cache weighing
const SCOPED_CIPHER_SIZE: usize = std::mem::size_of::<ScopedCipher>();

#[derive(Clone)]
pub struct ZeroKms {
    zerokms_client: Arc<ZerokmsClient>,
    cipher_cache: Cache<String, Arc<ScopedCipher>>,
}

impl ZeroKms {
    pub fn new(config: &TandemConfig) -> Result<Self, Error> {
        let zerokms_client = init_zerokms_client(config)?;

        let cipher_cache = Cache::builder()
            // Use weigher to calculate actual memory usage of ScopedCipher instances
            .weigher(|_key: &String, _value: &Arc<ScopedCipher>| -> u32 {
                SCOPED_CIPHER_SIZE as u32
            })
            // Set capacity in bytes (entry count * actual struct size)
            .max_capacity((config.server.cipher_cache_size as u64) * SCOPED_CIPHER_SIZE as u64)
            .time_to_live(Duration::from_secs(config.server.cipher_cache_ttl_seconds))
            .build();

        Ok(ZeroKms {
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
            debug!(target: "proxy", msg = "Use cached ScopedCipher", ?keyset_id);
            counter!(KEYSET_CIPHER_CACHE_HITS_TOTAL).increment(1);
            return Ok(cached_cipher);
        }

        let zerokms_client = self.zerokms_client.clone();

        debug!(target: "proxy", msg = "Initializing ZeroKMS ScopedCipher", ?keyset_id);

        let identified_by = keyset_id.as_ref().map(|id| id.0.clone());

        match ScopedCipher::init(zerokms_client, identified_by).await {
            Ok(cipher) => {
                let arc_cipher = Arc::new(cipher);

                counter!(KEYSET_CIPHER_INIT_TOTAL).increment(1);

                // Store in cache
                self.cipher_cache
                    .insert(cache_key, arc_cipher.clone())
                    .await;

                // Update pending tasks to get accurate cache statistics
                self.cipher_cache.run_pending_tasks().await;

                let entry_count = self.cipher_cache.entry_count();
                let memory_usage_bytes = self.cipher_cache.weighted_size();

                debug!(target: "proxy", msg = "ScopedCipher cached", ?keyset_id, entry_count, memory_usage_bytes);

                Ok(arc_cipher)
            }
            Err(err) => {
                debug!(target: "proxy", msg = "Error initializing ZeroKMS ScopedCipher", error = err.to_string());

                match err {
                    cipherstash_client::zerokms::Error::LoadKeyset(_) => {
                        Err(EncryptError::UnknownKeysetIdentifier {
                            keyset: keyset_id.map_or("default".to_string(), |id| id.to_string()),
                        }
                        .into())
                    }
                    _ => Err(err.into()),
                }
            }
        }
    }

    ///
    /// Encrypt `Plaintexts` using the `Column` configuration
    ///
    pub async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<Plaintext>>,
        columns: &[Option<Column>],
        default_keyset_id: Option<uuid::Uuid>,
    ) -> Result<Vec<Option<eql::EqlEncrypted>>, Error> {
        debug!(target: ENCRYPT, msg="Encrypt", ?keyset_id, ?default_keyset_id);

        // A keyset is required if no default keyset has been configured
        if default_keyset_id.is_none() && keyset_id.is_none() {
            return Err(EncryptError::MissingKeysetIdentifier.into());
        }

        let cipher = self.init_cipher(keyset_id).await?;

        let mut pipeline = ReferencedPendingPipeline::new(cipher.clone());
        let mut index_term_plaintexts = vec![None; columns.len()];

        for (idx, item) in plaintexts.into_iter().zip(columns.iter()).enumerate() {
            match item {
                (Some(plaintext), Some(column)) => {
                    if column.is_encryptable() {
                        let encryptable = PlaintextTarget::new(plaintext, column.config.clone());
                        pipeline.add_with_ref::<PlaintextTarget>(encryptable, idx)?;
                    } else {
                        index_term_plaintexts[idx] = Some(plaintext);
                    }
                }
                (None, Some(column)) => {
                    // Parameter is NULL
                    debug!(target: ENCRYPT, msg = "Null parameter", ?column);
                }
                (Some(plaintext), None) => {
                    // Should be unreachable
                    let plaintext_type = plaintext_type_name(plaintext);
                    return Err(EncryptError::MissingEncryptConfiguration { plaintext_type }.into());
                }
                (None, None) => {
                    // Parameter is not encryptable
                }
            }
        }

        let mut encrypted_eql = vec![];

        let mut result = pipeline.encrypt(None, None).await?;

        for (idx, opt) in columns.iter().enumerate() {
            let mut encrypted = None;

            if let Some(column) = opt {
                if let Some(e) = result.remove(idx) {
                    encrypted = Some(to_eql_encrypted(e, &column.identifier)?);
                } else if let Some(plaintext) = index_term_plaintexts[idx].clone() {
                    let index = column.config.clone().into_ste_vec_index().unwrap();
                    let op = QueryOp::SteVecSelector;

                    let index_term = (index, plaintext).build_queryable(cipher.clone(), op)?;

                    encrypted = Some(to_eql_encrypted_from_index_term(
                        index_term,
                        &column.identifier,
                    )?);
                }
            }

            encrypted_eql.push(encrypted);
        }

        Ok(encrypted_eql)
    }

    ///
    /// Decrypt eql::Ciphertext into Plaintext
    ///
    /// Database values are stored as `eql::Ciphertext`
    ///
    pub async fn decrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<eql::EqlEncrypted>>,
        default_keyset_id: Option<uuid::Uuid>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        // A keyset is required if no default keyset has been configured
        if default_keyset_id.is_none() && keyset_id.is_none() {
            return Err(EncryptError::MissingKeysetIdentifier.into());
        }
        debug!(target: ENCRYPT, msg="Decrypt", ?keyset_id, ?default_keyset_id);

        let cipher = self.init_cipher(keyset_id.clone()).await?;

        // Create a mutable vector to hold the decrypted results
        let mut results = vec![None; ciphertexts.len()];

        // Collect the index and ciphertext details for every Some(ciphertext)
        let (indices, encrypted): (Vec<_>, Vec<_>) = ciphertexts
            .into_iter()
            .enumerate()
            .filter_map(|(idx, eql)| Some((idx, eql?.body.ciphertext.unwrap())))
            .collect::<_>();

        let decrypted = cipher.decrypt(encrypted).await.map_err(|e| -> Error {
            match &e {
                cipherstash_client::zerokms::Error::Decrypt(_) => {
                    let error_msg = e.to_string();
                    if error_msg.contains("Failed to retrieve key") {
                        EncryptError::CouldNotDecryptDataForKeyset {
                            keyset_id: keyset_id
                                .map(|id| id.to_string())
                                .unwrap_or("default_keyset".to_string()),
                        }
                        .into()
                    } else {
                        e.into()
                    }
                }
                _ => e.into(),
            }
        })?;

        // Merge the decrypted values as plaintext into their original indexed positions
        for (idx, decrypted) in indices.into_iter().zip(decrypted) {
            let plaintext = Plaintext::from_slice(&decrypted)?;
            results[idx] = Some(plaintext);
        }

        Ok(results)
    }
}
