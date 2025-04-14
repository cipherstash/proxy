mod config;
mod schema;

use crate::{
    config::TandemConfig,
    connect, eql,
    error::{EncryptError, Error},
    log::ENCRYPT,
    postgresql::Column,
    Identifier,
};
use cipherstash_client::{
    config::EnvSource,
    credentials::{auto_refresh::AutoRefresh, ServiceCredentials},
    encryption::{
        self, Encrypted, EncryptionError, IndexTerm, Plaintext, PlaintextTarget,
        ReferencedPendingPipeline,
    },
    zerokms::EncryptedRecord,
    ConsoleConfig, CtsConfig, ZeroKMSConfig,
};
use cipherstash_config::ColumnConfig;
use config::EncryptConfigManager;
use schema::SchemaManager;
use std::{sync::Arc, vec};
use tracing::{debug, warn};

/// SQL Statement for loading encrypt configuration from database
const ENCRYPT_CONFIG_QUERY: &str = include_str!("./sql/select_config.sql");

/// SQL Statement for loading database schema
const SCHEMA_QUERY: &str = include_str!("./sql/select_table_schemas.sql");

/// SQL Statement for loading aggregates as part of database schema
const AGGREGATE_QUERY: &str = include_str!("./sql/select_aggregates.sql");

type ScopedCipher = encryption::ScopedCipher<AutoRefresh<ServiceCredentials>>;

///
/// All of the things required for Encrypt-as-a-Product
///
#[derive(Debug, Clone)]
pub struct Encrypt {
    pub config: TandemConfig,
    cipher: Arc<ScopedCipher>,
    pub encrypt_config: EncryptConfigManager,
    pub schema: SchemaManager,
    /// The EQL version installed in the database or `None` if it was not present
    pub eql_version: Option<String>,
}

impl Encrypt {
    pub async fn init(config: TandemConfig) -> Result<Encrypt, Error> {
        let cipher = Arc::new(init_cipher(&config).await?);
        let schema = SchemaManager::init(&config.database).await?;
        let encrypt_config = EncryptConfigManager::init(&config.database).await?;

        let eql_version = {
            let client = connect::database(&config.database).await?;
            let rows = client.query("SELECT cs_eql_version();", &[]).await;

            match rows {
                Ok(rows) => rows.first().map(|row| row.get("cs_eql_version")),
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
            cipher,
            encrypt_config,
            schema,
            eql_version,
        })
    }

    ///
    /// Encrypt `Plaintexts` using the `Column` configuration
    ///
    ///
    pub async fn encrypt(
        &self,
        plaintexts: Vec<Option<Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<eql::Encrypted>>, Error> {
        let mut pipeline = ReferencedPendingPipeline::new(self.cipher.clone());

        for (idx, item) in plaintexts.into_iter().zip(columns.iter()).enumerate() {
            match item {
                (Some(plaintext), Some(column)) => {
                    let encryptable = PlaintextTarget::new(plaintext, column.config.clone());
                    pipeline.add_with_ref::<PlaintextTarget>(encryptable, idx)?;
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
        if !pipeline.is_empty() {
            let mut result = pipeline.encrypt(None).await?;

            for (idx, opt) in columns.iter().enumerate() {
                let mut encrypted = None;
                if let Some(col) = opt {
                    if let Some(e) = result.remove(idx) {
                        encrypted = Some(to_eql_encrypted(e, &col.identifier)?);
                    }
                }
                encrypted_eql.push(encrypted);
            }
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
        ciphertexts: Vec<Option<eql::Encrypted>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        // Create a mutable vector to hold the decrypted results
        let mut results = vec![None; ciphertexts.len()];

        // Collect the index and ciphertext details for every Some(ciphertext)
        let (indices, encrypted) = ciphertexts
            .into_iter()
            .enumerate()
            .filter_map(|(idx, opt)| {
                opt.map(|ct| {
                    eql_encrypted_to_encrypted_record(ct)
                        .map(|encrypted_record| (idx, encrypted_record))
                })
            })
            .collect::<Result<(Vec<_>, Vec<_>), _>>()?;

        // Decrypt the ciphertexts
        let decrypted = self.cipher.decrypt(encrypted).await?;

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

async fn init_cipher(config: &TandemConfig) -> Result<ScopedCipher, Error> {
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
        .workspace_id(&config.auth.workspace_id)
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

    let zerokms_config = builder.build_with_client_key()?;

    let zerokms_client = zerokms_config
        .create_client_with_credentials(AutoRefresh::new(zerokms_config.credentials()));

    match ScopedCipher::init(Arc::new(zerokms_client), config.encrypt.default_keyset_id).await {
        Ok(cipher) => {
            debug!(target: ENCRYPT, msg = "Initialized ZeroKMS ScopedCipher");
            Ok(cipher)
        }
        Err(err) => {
            debug!(target: ENCRYPT, msg =  "Error initializing ZeroKMS ScopedCipher", error = err.to_string());
            Err(err.into())
        }
    }
}

fn to_eql_encrypted(
    encrypted: Encrypted,
    identifier: &Identifier,
) -> Result<eql::Encrypted, Error> {
    debug!(target: ENCRYPT, msg = "Encrypted to EQL", ?identifier);
    match encrypted {
        Encrypted::Record(ciphertext, terms) => {
            struct Indexes {
                match_index: Option<Vec<u16>>,
                ore_index: Option<Vec<String>>,
                unique_index: Option<String>,
            }

            let mut indexes = Indexes {
                match_index: None,
                ore_index: None,
                unique_index: None,
            };

            for index_term in terms {
                match index_term {
                    IndexTerm::Binary(bytes) => {
                        indexes.unique_index = Some(format_index_term_binary(&bytes))
                    }
                    IndexTerm::BitMap(inner) => indexes.match_index = Some(inner),
                    IndexTerm::OreArray(vec_of_bytes) => {
                        indexes.ore_index = Some(format_index_term_ore_array(&vec_of_bytes));
                    }
                    IndexTerm::OreFull(bytes) => {
                        indexes.ore_index = Some(format_index_term_ore(&bytes));
                    }
                    IndexTerm::OreLeft(bytes) => {
                        indexes.ore_index = Some(format_index_term_ore(&bytes));
                    }
                    IndexTerm::Null => {}
                    _ => return Err(EncryptError::UnknownIndexTerm(identifier.to_owned()).into()),
                };
            }

            Ok(eql::Encrypted::Ciphertext {
                ciphertext,
                identifier: identifier.to_owned(),
                match_index: indexes.match_index,
                ore_index: indexes.ore_index,
                unique_index: indexes.unique_index,
                version: 1,
            })
        }
        Encrypted::SteVec(ste_vec_index) => Ok(eql::Encrypted::SteVec {
            identifier: identifier.to_owned(),
            ste_vec_index,
            version: 1,
        }),
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

fn eql_encrypted_to_encrypted_record(
    eql_encrypted: eql::Encrypted,
) -> Result<EncryptedRecord, EncryptionError> {
    match eql_encrypted {
        eql::Encrypted::Ciphertext { ciphertext, .. } => Ok(ciphertext),
        eql::Encrypted::SteVec { ste_vec_index, .. } => ste_vec_index.into_root_ciphertext(),
    }
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
