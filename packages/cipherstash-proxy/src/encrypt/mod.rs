use crate::{
    config::{EncryptConfigManager, SchemaManager, TandemConfig},
    eql,
    error::{EncryptError, Error},
    log::{DEVELOPMENT, ENCRYPT},
    postgresql::Column,
    Identifier,
};
use cipherstash_client::{
    credentials::{auto_refresh::AutoRefresh, ServiceCredentials},
    encryption::{
        self, Encrypted, IndexTerm, Plaintext, PlaintextTarget, ReferencedPendingPipeline,
    },
    ConsoleConfig, CtsConfig, ZeroKMSConfig,
};
use cipherstash_config::ColumnConfig;
use std::{sync::Arc, vec};
use tracing::debug;

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
}

impl Encrypt {
    pub async fn init(config: TandemConfig) -> Result<Encrypt, Error> {
        let cipher = Arc::new(init_cipher(&config).await?);
        let schema = SchemaManager::init(&config.database).await?;
        let encrypt_config = EncryptConfigManager::init(&config.database).await?;

        Ok(Encrypt {
            config,
            cipher,
            encrypt_config,
            schema,
        })
    }

    ///
    /// Encrypt `Plaintexts` using the `Column` configuration
    ///
    ///
    pub async fn encrypt(
        &self,
        plaintexts: Vec<Plaintext>,
        columns: &[Column],
    ) -> Result<Vec<eql::Ciphertext>, Error> {
        let mut pipeline = ReferencedPendingPipeline::new(self.cipher.clone());

        for (idx, (plaintext, column)) in plaintexts.into_iter().zip(columns.iter()).enumerate() {
            let encryptable = PlaintextTarget::new(plaintext, column.config.clone());
            pipeline.add_with_ref::<PlaintextTarget>(encryptable, idx)?;
        }

        let mut encrypted_eql = vec![];
        if !pipeline.is_empty() {
            let mut result = pipeline.encrypt(None).await?;

            for (idx, col) in columns.iter().enumerate() {
                let maybe_encrypted = result.remove(idx);
                match maybe_encrypted {
                    Some(encrypted) => {
                        let ct = to_eql_encrypted(encrypted, &col.identifier)?;
                        encrypted_eql.push(ct);
                    }
                    None => {
                        return Err(EncryptError::ColumnNotEncrypted {
                            table: col.identifier.table.to_string(),
                            column: col.identifier.column.to_string(),
                        }
                        .into());
                    }
                }
            }
        }

        Ok(encrypted_eql)
    }

    ///
    /// Encrypt `Plaintexts` using the `Column` configuration
    ///
    ///
    pub async fn encrypt_some(
        &self,
        plaintexts: Vec<Option<Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<eql::Ciphertext>>, Error> {
        let mut pipeline = ReferencedPendingPipeline::new(self.cipher.clone());

        // Zip the plaintexts and columns together
        // For each plaintex/column pair, create a new PlaintextTarget
        let received = plaintexts.len();

        for (idx, item) in plaintexts.into_iter().zip(columns.iter()).enumerate() {
            match item {
                (Some(plaintext), Some(column)) => {
                    let encryptable = PlaintextTarget::new(plaintext, column.config.clone());
                    pipeline.add_with_ref::<PlaintextTarget>(encryptable, idx)?;
                }
                (None, None) => {
                    // Parameter is not encryptable
                    // Do nothing
                }
                _ => {
                    return Err(EncryptError::EncryptedColumnMismatch {
                        expected: columns.len(),
                        received,
                    }
                    .into());
                }
            }
        }

        let mut encrypted_eql = vec![];
        if !pipeline.is_empty() {
            let mut result = pipeline.encrypt(None).await?;

            for (idx, opt) in columns.iter().enumerate() {
                match opt {
                    Some(col) => {
                        let maybe_encrypted = result.remove(idx);
                        match maybe_encrypted {
                            Some(encrypted) => {
                                let ct = to_eql_encrypted(encrypted, &col.identifier)?;
                                encrypted_eql.push(Some(ct));
                            }
                            None => {
                                return Err(EncryptError::ColumnNotEncrypted {
                                    table: col.identifier.table.to_string(),
                                    column: col.identifier.column.to_string(),
                                }
                                .into());
                            }
                        }
                    }
                    None => encrypted_eql.push(None),
                }
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
        ciphertexts: Vec<Option<eql::Ciphertext>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        // Create a mutable vector to hold the decrypted results
        let mut results = vec![None; ciphertexts.len()];

        // Collect the index and ciphertext details for every Some(ciphertext)
        let (indices, encrypted): (Vec<_>, Vec<_>) = ciphertexts
            .into_iter()
            .enumerate()
            .filter_map(|(idx, opt)| opt.map(|ct| (idx, ct.ciphertext)))
            .unzip();

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
        self.schema.reload().await
    }

    pub fn is_passthrough(&self) -> bool {
        self.encrypt_config.is_empty() || self.config.disable_mapping()
    }

    pub fn is_empty_config(&self) -> bool {
        self.encrypt_config.is_empty()
    }
}

async fn init_cipher(config: &TandemConfig) -> Result<ScopedCipher, Error> {
    let console_config = ConsoleConfig::builder().with_env().build()?;
    let cts_config = CtsConfig::builder().with_env().build()?;

    // Not using with_env because the proxy config should take precedence
    let builder = ZeroKMSConfig::builder(); //.with_env();

    let zerokms_config = builder
        .workspace_id(&config.auth.workspace_id)
        .access_key(&config.auth.client_access_key)
        .client_id(&config.encrypt.client_id)
        .client_key(&config.encrypt.client_key)
        .console_config(&console_config)
        .cts_config(&cts_config)
        .build_with_client_key()?;

    let zerokms_client = zerokms_config
        .create_client_with_credentials(AutoRefresh::new(zerokms_config.credentials()));

    match ScopedCipher::init(Arc::new(zerokms_client), config.encrypt.dataset_id).await {
        Ok(cipher) => {
            debug!(target: DEVELOPMENT, msg = "Initialized ZeroKMS ScopedCipher");
            Ok(cipher)
        }
        Err(err) => {
            debug!(target: DEVELOPMENT, msg =  "Error initializing ZeroKMS ScopedCipher", error = err.to_string());
            Err(err.into())
        }
    }
}

fn to_eql_encrypted(
    encrypted: Encrypted,
    identifier: &Identifier,
) -> Result<eql::Ciphertext, Error> {
    match encrypted {
        Encrypted::Record(ciphertext, terms) => {
            debug!(target: ENCRYPT, ciphertext = ?ciphertext);
            debug!(target: ENCRYPT, terms = ?terms);

            let mut ciphertext = eql::Ciphertext::new(ciphertext, identifier.to_owned());

            for index_term in terms {
                match index_term {
                    IndexTerm::Binary(bytes) => {
                        ciphertext.unique_index = Some(format_index_term_binary(&bytes))
                    }
                    IndexTerm::BitMap(inner) => ciphertext.match_index = Some(inner),
                    IndexTerm::OreArray(vec_of_bytes) => {
                        ciphertext.ore_index = Some(format_index_term_ore_array(&vec_of_bytes));
                    }
                    IndexTerm::OreFull(bytes) => {
                        ciphertext.ore_index = Some(format_index_term_ore_full(&bytes));
                    }
                    IndexTerm::OreLeft(bytes) => {
                        ciphertext.ore_index = Some(format_index_term_ore_full(&bytes));
                    }
                    IndexTerm::Null => {}
                    _ => return Err(EncryptError::UnknownIndexTerm(identifier.to_owned()).into()),
                };
            }
            Ok(ciphertext)
        }
        Encrypted::SteVec(_ste_vec_index) => {
            todo!("Encrypted::SteVec");
        }
    }
}

fn format_index_term_ore(bytes: &Vec<u8>) -> String {
    format!(
        "{}{}{}",
        r#"""(\\""\\\\\\\\x"#,
        hex::encode(bytes),
        r#"\\"")"""#
    )
}

fn format_index_term_ore_full(bytes: &Vec<u8>) -> String {
    format!("{}{}{}", r#"("{"#, format_index_term_ore(bytes), r#"}")"#)
}

fn format_index_term_binary(bytes: &Vec<u8>) -> String {
    hex::encode(bytes)
}

fn format_index_term_ore_array(vec_of_bytes: &[Vec<u8>]) -> String {
    let inner: Vec<String> = vec_of_bytes.iter().map(format_index_term_ore).collect();
    format!("{}{}{}", r#"("{"#, inner.join(", "), r#"}")"#)
}
