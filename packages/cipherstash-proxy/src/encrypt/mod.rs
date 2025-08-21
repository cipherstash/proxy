mod config;
mod schema;

use crate::{
    config::TandemConfig,
    connect,
    eql::{self, EqlEncryptedBody, EqlEncryptedIndexes},
    error::{EncryptError, Error},
    log::ENCRYPT,
    postgresql::{Column, KeysetIdentifier},
    Identifier, EQL_SCHEMA_VERSION,
};
use cipherstash_client::{
    config::EnvSource,
    credentials::{auto_refresh::AutoRefresh, ServiceCredentials},
    encryption::{
        self, Encrypted, EncryptedEntry, EncryptedSteVecTerm, IndexTerm, Plaintext,
        PlaintextTarget, Queryable, ReferencedPendingPipeline,
    },
    schema::ColumnConfig,
    zerokms::ClientKey,
    ConsoleConfig, CtsConfig, ZeroKMS, ZeroKMSConfig,
};
use cipherstash_client::{
    config::{ConfigError, ZeroKMSConfigWithClientKey},
    encryption::QueryOp,
};
use config::EncryptConfigManager;
use schema::SchemaManager;
use std::{sync::Arc, vec};
use tracing::{debug, info, warn};

/// SQL Statement for loading encrypt configuration from database
const ENCRYPT_CONFIG_QUERY: &str = include_str!("./sql/select_config.sql");

/// SQL Statement for loading database schema
const SCHEMA_QUERY: &str = include_str!("./sql/select_table_schemas.sql");

/// SQL Statement for loading aggregates as part of database schema
const AGGREGATE_QUERY: &str = include_str!("./sql/select_aggregates.sql");

type ScopedCipher = encryption::ScopedCipher<AutoRefresh<ServiceCredentials>>;

type ZerokmsClient = ZeroKMS<AutoRefresh<ServiceCredentials>, ClientKey>;

///
/// All of the things required for Encrypt-as-a-Product
///
#[derive(Clone)]
pub struct Encrypt {
    pub config: TandemConfig,
    pub encrypt_config: EncryptConfigManager,
    pub schema: SchemaManager,
    /// The EQL version installed in the database or `None` if it was not present
    pub eql_version: Option<String>,
    zerokms_client: Arc<ZerokmsClient>,
}

impl Encrypt {
    pub async fn init(config: TandemConfig) -> Result<Encrypt, Error> {
        let zerokms_client = init_zerokms_client(&config)?;

        let encrypt_config = EncryptConfigManager::init(&config.database).await?;
        // TODO: populate EqlTraitImpls based in config
        let schema = SchemaManager::init(&config.database).await?;

        let eql_version = {
            let client = connect::database(&config.database).await?;
            let rows = client
                .query("SELECT eql_v2.version() AS version;", &[])
                .await;

            match rows {
                Ok(rows) => rows.first().map(|row| row.get("version")),
                Err(err) => {
                    warn!(
                        msg = "Could not query EQL version from database",
                        error = err.to_string()
                    );
                    None
                }
            }
        };

        Ok(Encrypt {
            config,
            zerokms_client: Arc::new(zerokms_client),
            encrypt_config,
            schema,
            eql_version,
        })
    }

    /// Initialize cipher using the stored zerokms_config
    pub async fn init_cipher(
        &self,
        keyset_id: Option<KeysetIdentifier>,
    ) -> Result<ScopedCipher, Error> {
        let zerokms_client = self.zerokms_client.clone();

        debug!(target: ENCRYPT, msg = "Initializing ZeroKMS ScopedCipher", ?keyset_id);

        let identified_by = keyset_id.clone().map(|id| id.0);

        match ScopedCipher::init(zerokms_client, identified_by).await {
            Ok(cipher) => Ok(cipher),
            Err(err) => {
                debug!(target: ENCRYPT, msg = "Error initializing ZeroKMS ScopedCipher", error = err.to_string());

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
    ///
    pub async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<eql::EqlEncrypted>>, Error> {
        debug!(target: ENCRYPT, msg="Encrypt", ?keyset_id, default_keyset_id = ?self.config.encrypt.default_keyset_id);

        // A keyset is required if no default keyset has been configured
        if self.config.encrypt.default_keyset_id.is_none() && keyset_id.is_none() {
            return Err(EncryptError::MissingKeysetIdentifier.into());
        }

        let cipher = Arc::new(self.init_cipher(keyset_id).await?);

        let mut pipeline = ReferencedPendingPipeline::new(cipher.clone());
        let mut index_term_plaintexts = vec![None; columns.len()];

        for (idx, item) in plaintexts.into_iter().zip(columns.iter()).enumerate() {
            match item {
                (Some(plaintext), Some(column)) => {
                    info!(target: ENCRYPT, msg = "ENCRYPT", idx, ?column, ?plaintext);

                    if column.is_encryptable() {
                        let encryptable = PlaintextTarget::new(plaintext, column.config.clone());
                        pipeline.add_with_ref::<PlaintextTarget>(encryptable, idx)?;
                    } else {
                        info!(target: ENCRYPT, msg = "Add to index_term_plaintexts", idx, ?column);
                        index_term_plaintexts[idx] = Some(plaintext);
                    }
                }
                (None, Some(column)) => {
                    // Parameter is NULL
                    // Do nothing
                    debug!(target: ENCRYPT, msg = "Null parameter", ?column);
                }
                (Some(plaintext), None) => {
                    // Should be unreachable
                    // Bind doesn't know what type of Plaintext to create in the first place if the column is None
                    let plaintext_type = plaintext_type_name(plaintext);
                    return Err(EncryptError::MissingEncryptConfiguration { plaintext_type }.into());
                }
                (None, None) => {
                    // Parameter is not encryptable
                    // Do nothing
                }
            }
        }

        let mut encrypted_eql = vec![];
        // if !pipeline.is_empty() { }

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
    ///
    pub async fn decrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<eql::EqlEncrypted>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        // A keyset is required if no default keyset has been configured
        if self.config.encrypt.default_keyset_id.is_none() && keyset_id.is_none() {
            return Err(EncryptError::MissingKeysetIdentifier.into());
        }
        debug!(target: ENCRYPT, msg="Decrypt", ?keyset_id);

        let cipher = Arc::new(self.init_cipher(keyset_id.clone()).await?);

        // Create a mutable vector to hold the decrypted results
        let mut results = vec![None; ciphertexts.len()];

        // Collect the index and ciphertext details for every Some(ciphertext)
        let (indices, encrypted): (Vec<_>, Vec<_>) = ciphertexts
            .into_iter()
            .enumerate()
            .filter_map(|(idx, eql)| Some((idx, eql?.body.ciphertext.unwrap())))
            .collect::<_>();

        debug!(target: ENCRYPT, ?encrypted);

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

    pub fn get_column_config(&self, identifier: &eql::Identifier) -> Option<ColumnConfig> {
        let encrypt_config = self.encrypt_config.load();
        encrypt_config.get(identifier).cloned()
    }

    pub async fn reload_schema(&self) {
        self.schema.reload().await;
        self.encrypt_config.reload().await;
    }

    pub fn is_passthrough(&self) -> bool {
        self.encrypt_config.is_empty() || self.config.mapping_disabled()
    }

    pub fn is_empty_config(&self) -> bool {
        self.encrypt_config.is_empty()
    }
}

fn init_zerokms_client(
    config: &TandemConfig,
) -> Result<ZeroKMS<AutoRefresh<ServiceCredentials>, ClientKey>, ConfigError> {
    let zerokms_config = build_zerokms_config(config)?;

    Ok(zerokms_config
        .create_client_with_credentials(AutoRefresh::new(zerokms_config.credentials())))
}

fn build_zerokms_config(config: &TandemConfig) -> Result<ZeroKMSConfigWithClientKey, ConfigError> {
    let console_config = ConsoleConfig::builder().with_env().build()?;

    let builder = CtsConfig::builder().with_env();
    let builder = if let Some(cts_host) = config.cts_host() {
        builder.base_url(&cts_host)
    } else {
        builder
    };
    let cts_config = builder.build()?;

    // Not using with_env because the proxy config should take precedence
    let builder = ZeroKMSConfig::builder()
        .add_source(EnvSource::default())
        .workspace_crn(config.auth.workspace_crn.clone())
        .access_key(&config.auth.client_access_key)
        .try_with_client_id(&config.encrypt.client_id)?
        .try_with_client_key(&config.encrypt.client_key)?
        .console_config(&console_config)
        .cts_config(&cts_config);

    let builder = if let Some(zerokms_host) = config.zerokms_host() {
        builder.base_url(zerokms_host)
    } else {
        builder
    };

    builder.build_with_client_key()
}

fn to_eql_encrypted_from_index_term(
    index_term: IndexTerm,
    identifier: &Identifier,
) -> Result<eql::EqlEncrypted, Error> {
    debug!(target: ENCRYPT, msg = "Encrypted to EQL", ?identifier);

    let selector = match index_term {
        IndexTerm::SteVecSelector(s) => Some(hex::encode(s.as_bytes())),
        _ => return Err(EncryptError::InvalidIndexTerm.into()),
    };

    Ok(eql::EqlEncrypted {
        identifier: identifier.to_owned(),
        version: EQL_SCHEMA_VERSION,
        body: EqlEncryptedBody {
            ciphertext: None,
            indexes: EqlEncryptedIndexes {
                bloom_filter: None,
                ore_block_u64_8_256: None,
                hmac_256: None,
                blake3: None,
                ore_cllw_u64_8: None,
                ore_cllw_var_8: None,
                selector,
                ste_vec_index: None,
            },
            is_array_item: None,
        },
    })
}

fn to_eql_encrypted(
    encrypted: Encrypted,
    identifier: &Identifier,
) -> Result<eql::EqlEncrypted, Error> {
    debug!(target: ENCRYPT, msg = "Encrypted to EQL", ?identifier);

    match encrypted {
        Encrypted::Record(ciphertext, terms) => {
            let mut match_index: Option<Vec<u16>> = None;
            let mut ore_index: Option<Vec<String>> = None;
            let mut unique_index: Option<String> = None;
            let mut blake3_index: Option<String> = None;
            let mut ore_cclw_fixed_index: Option<String> = None;
            let mut ore_cclw_var_index: Option<String> = None;
            let mut selector: Option<String> = None;

            for index_term in terms {
                match index_term {
                    IndexTerm::Binary(bytes) => {
                        unique_index = Some(format_index_term_binary(&bytes))
                    }
                    IndexTerm::BitMap(inner) => match_index = Some(inner),
                    IndexTerm::OreArray(bytes) => {
                        ore_index = Some(format_index_term_ore_array(&bytes));
                    }
                    IndexTerm::OreFull(bytes) => {
                        ore_index = Some(format_index_term_ore(&bytes));
                    }
                    IndexTerm::OreLeft(bytes) => {
                        ore_index = Some(format_index_term_ore(&bytes));
                    }
                    IndexTerm::BinaryVec(_) => todo!(),
                    IndexTerm::SteVecSelector(s) => {
                        selector = Some(hex::encode(s.as_bytes()));
                    }
                    IndexTerm::SteVecTerm(ste_vec_term) => match ste_vec_term {
                        EncryptedSteVecTerm::Mac(bytes) => blake3_index = Some(hex::encode(bytes)),
                        EncryptedSteVecTerm::OreFixed(ore) => {
                            ore_cclw_fixed_index = Some(hex::encode(&ore))
                        }
                        EncryptedSteVecTerm::OreVariable(ore) => {
                            ore_cclw_var_index = Some(hex::encode(&ore))
                        }
                    },
                    IndexTerm::SteQueryVec(_query) => {} // TODO: what do we do here?
                    IndexTerm::Null => {}
                };
            }

            Ok(eql::EqlEncrypted {
                identifier: identifier.to_owned(),
                version: EQL_SCHEMA_VERSION,
                body: EqlEncryptedBody {
                    ciphertext: Some(ciphertext),
                    indexes: EqlEncryptedIndexes {
                        bloom_filter: match_index,
                        ore_block_u64_8_256: ore_index,
                        hmac_256: unique_index,
                        blake3: blake3_index,
                        ore_cllw_u64_8: ore_cclw_fixed_index,
                        ore_cllw_var_8: ore_cclw_var_index,
                        selector,
                        ste_vec_index: None,
                    },
                    is_array_item: None,
                },
            })
        }
        Encrypted::SteVec(ste_vec) => {
            let ciphertext = ste_vec.root_ciphertext()?.clone();

            let ste_vec_index: Vec<EqlEncryptedBody> = ste_vec
                .into_iter()
                .map(
                    |EncryptedEntry {
                         tokenized_selector,
                         term,
                         record,
                         parent_is_array,
                     }| {
                        let indexes = match term {
                            EncryptedSteVecTerm::Mac(bytes) => EqlEncryptedIndexes {
                                selector: Some(hex::encode(tokenized_selector.as_bytes())),
                                blake3: Some(hex::encode(bytes)),
                                ..Default::default()
                            },
                            EncryptedSteVecTerm::OreFixed(ore) => EqlEncryptedIndexes {
                                selector: Some(hex::encode(tokenized_selector.as_bytes())),
                                ore_cllw_u64_8: Some(hex::encode(&ore)),
                                ..Default::default()
                            },
                            EncryptedSteVecTerm::OreVariable(ore) => EqlEncryptedIndexes {
                                selector: Some(hex::encode(tokenized_selector.as_bytes())),
                                ore_cllw_var_8: Some(hex::encode(&ore)),
                                ..Default::default()
                            },
                        };

                        eql::EqlEncryptedBody {
                            ciphertext: Some(record),
                            indexes,
                            is_array_item: Some(parent_is_array),
                        }
                    },
                )
                .collect();

            // FIXME: I'm unsure if I've handled the root ciphertext correctly
            // The way it's implemented right now is that it will be repeated one in the ste_vec_index.
            Ok(eql::EqlEncrypted {
                identifier: identifier.to_owned(),
                version: EQL_SCHEMA_VERSION,
                body: EqlEncryptedBody {
                    ciphertext: Some(ciphertext.clone()),
                    indexes: EqlEncryptedIndexes {
                        bloom_filter: None,
                        ore_block_u64_8_256: None,
                        hmac_256: None,
                        blake3: None,
                        ore_cllw_u64_8: None,
                        ore_cllw_var_8: None,
                        selector: None,
                        ste_vec_index: Some(ste_vec_index),
                    },
                    is_array_item: None,
                },
            })
        }
    }
}

fn format_index_term_binary(bytes: &Vec<u8>) -> String {
    hex::encode(bytes)
}

fn format_index_term_ore_bytea(bytes: &Vec<u8>) -> String {
    hex::encode(bytes)
}

///
/// Formats a Vec<Vec<u8>> into a Vec<String>
///
fn format_index_term_ore_array(vec_of_bytes: &[Vec<u8>]) -> Vec<String> {
    vec_of_bytes
        .iter()
        .map(format_index_term_ore_bytea)
        .collect()
}

///
/// Formats a Vec<Vec<u8>> into a single elenent Vec<String>
///
fn format_index_term_ore(bytes: &Vec<u8>) -> Vec<String> {
    vec![format_index_term_ore_bytea(bytes)]
}

fn plaintext_type_name(pt: Plaintext) -> String {
    match pt {
        Plaintext::BigInt(_) => "BigInt".to_string(),
        Plaintext::BigUInt(_) => "BigUInt".to_string(),
        Plaintext::Boolean(_) => "Boolean".to_string(),
        Plaintext::Decimal(_) => "Decimal".to_string(),
        Plaintext::Float(_) => "Float".to_string(),
        Plaintext::Int(_) => "Int".to_string(),
        Plaintext::NaiveDate(_) => "NaiveDate".to_string(),
        Plaintext::SmallInt(_) => "SmallInt".to_string(),
        Plaintext::Timestamp(_) => "Timestamp".to_string(),
        Plaintext::Utf8Str(_) => "Utf8Str".to_string(),
        Plaintext::JsonB(_) => "JsonB".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::with_no_cs_vars;
    use cts_common::WorkspaceId;

    fn build_tandem_config(env: Vec<(&str, Option<&str>)>) -> TandemConfig {
        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                TandemConfig::build("tests/config/unknown.toml").unwrap()
            })
        })
    }

    fn default_env_vars() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("CS_DATABASE__USERNAME", Some("postgres")),
            ("CS_DATABASE__PASSWORD", Some("password")),
            ("CS_DATABASE__NAME", Some("db")),
            ("CS_DATABASE__HOST", Some("localhost")),
            ("CS_DATABASE__PORT", Some("5432")),
            ("CS_ENCRYPT__KEYSET_ID", Some("c50d8463-60e9-41a5-86cd-5782e03a503c")),
            ("CS_ENCRYPT__CLIENT_ID", Some("e40f1692-6bb7-4bbd-a552-4c0f155be073")),
            ("CS_ENCRYPT__CLIENT_KEY", Some("a4627031a16b7065726d75746174696f6e90090e0805000b0d0c0106040f0a0302076770325f66726f6da16b7065726d75746174696f6e9007060a0b02090d080c00040f0305010e6570325f746fa16b7065726d75746174696f6e900a0206090b04050c070f0e010d030800627033a16b7065726d75746174696f6e98210514181d0818200a18190b1112181809130f15181a0717181e000e0103181f0d181c1602040c181b1006")),
        ]
    }

    #[test]
    fn build_zerokms_config_with_crn() {
        with_no_cs_vars(|| {
            let mut env = default_env_vars();
            env.push(("CS_CLIENT_ACCESS_KEY", Some("client-access-key")));
            env.push((
                "CS_WORKSPACE_CRN",
                Some("crn:ap-southeast-2.aws:3KISDURL3ZCWYZ2O"),
            ));

            let tandem_config = build_tandem_config(env);

            let zerokms_config = build_zerokms_config(&tandem_config).unwrap();

            assert_eq!(
                WorkspaceId::try_from("3KISDURL3ZCWYZ2O").unwrap(),
                zerokms_config.workspace_id()
            );

            assert!(zerokms_config
                .base_url()
                .to_string()
                .contains("ap-southeast-2.aws"));
        });
    }
}
