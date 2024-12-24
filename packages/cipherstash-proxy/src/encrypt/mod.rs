use crate::{
    config::{EncryptConfigManager, SchemaManager, TandemConfig},
    eql,
    error::{EncryptError, Error},
    postgresql::Column,
    Identifier,
};
use cipherstash_client::{
    credentials::{auto_refresh::AutoRefresh, service_credentials::ServiceCredentials},
    encryption::{self, Encrypted, Plaintext, PlaintextTarget, ReferencedPendingPipeline},
    ConsoleConfig, CtsConfig, ZeroKMS, ZeroKMSConfig,
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

    pub async fn encrypt(
        &self,
        plaintexts: Vec<Option<Plaintext>>,
        columns: Vec<Option<Column>>,
    ) -> Result<Vec<Option<eql::Ciphertext>>, Error> {
        let mut pipeline = ReferencedPendingPipeline::new(self.cipher.clone());

        plaintexts
            .into_iter()
            .zip(columns.clone())
            .enumerate()
            .for_each(|(idx, item)| match item {
                (Some(plaintext), Some(column)) => {
                    let encryptable = PlaintextTarget::new(plaintext, column.config, None);
                    pipeline
                        .add_with_ref::<PlaintextTarget>(encryptable, idx)
                        .unwrap();
                }
                (None, Some(_)) => todo!(),
                (Some(_), None) => todo!(),
                (None, None) => {
                    // Do nothing
                    // Parameter is not encrytptable
                }
            });

        let mut encrypted_eql = vec![];
        if !pipeline.is_empty() {
            let mut result = pipeline.encrypt().await?;

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

    pub async fn decrypt(
        &self,
        ciphertexts: Vec<Option<eql::Ciphertext>>,
    ) -> Result<Vec<Option<eql::Plaintext>>, Error> {
        // Create a mutable vector to hold the decrypted results
        let mut results = vec![None; ciphertexts.len()];

        // Collect the index and ciphertext details for every Some(ciphertext)
        let (indices, encrypted): (Vec<_>, Vec<_>) = ciphertexts
            .into_iter()
            .enumerate()
            .filter_map(|(idx, opt)| {
                opt.map(|ct| ((idx, ct.identifier, ct.version), ct.ciphertext))
            })
            .unzip();

        // Decrypt the ciphertexts
        let decrypted = self.cipher.decrypt(encrypted).await?;

        // Merge the decrypted values as plaintext into their original indexed positions
        for ((idx, identifier, version), decrypted) in indices.into_iter().zip(decrypted) {
            let plaintext = Plaintext::from_slice(&decrypted[..])?;

            let plaintext = match &plaintext {
                Plaintext::Utf8Str(Some(s)) => s.to_owned(),
                _ => todo!(),
                // Plaintext::BigInt(_) => todo!(),
                // Plaintext::BigUInt(_) => todo!(),
                // Plaintext::Boolean(_) => todo!(),
                // Plaintext::Decimal(decimal) => todo!(),
                // Plaintext::Float(_) => todo!(),
                // Plaintext::Int(_) => todo!(),
                // Plaintext::NaiveDate(naive_date) => todo!(),
                // Plaintext::SmallInt(_) => todo!(),
                // Plaintext::Timestamp(date_time) => todo!(),
                // Plaintext::JsonB(value) => todo!(),
            };
            results[idx] = Some(eql::Plaintext {
                plaintext,
                identifier,
                version,
                for_query: None,
            });
        }

        Ok(results)
    }

    pub fn get_column_config(&self, identifier: &eql::Identifier) -> Option<ColumnConfig> {
        let encrypt_config = self.encrypt_config.load();

        match encrypt_config.get(identifier) {
            Some(c) => Some(c.clone()),
            None => None,
        }
    }
}

async fn init_cipher(config: &TandemConfig) -> Result<ScopedCipher, Error> {
    let console_config = ConsoleConfig::builder().with_env().build()?;
    let cts_config = CtsConfig::builder().with_env().build()?;

    let builder = ZeroKMSConfig::builder().with_env();

    let zerokms_config = builder
        .workspace_id(&config.auth.workspace_id)
        .access_key(&config.auth.client_access_key)
        .client_id(&config.encrypt.client_id)
        .client_key(&config.encrypt.client_key)
        .console_config(&console_config)
        .cts_config(&cts_config)
        .build_with_client_key()?;

    // Build ZeroKMS client with client key manually
    // because create_client() doesn't support AutoRefresh
    let zerokms_client = ZeroKMS::new_with_client_key(
        &zerokms_config.base_url(),
        AutoRefresh::new(zerokms_config.credentials()),
        zerokms_config.decryption_log_path().as_deref(),
        zerokms_config.client_key(),
    );

    match ScopedCipher::init(Arc::new(zerokms_client), config.encrypt.dataset_id).await {
        Ok(cipher) => {
            debug!("Initialized ZeroKMS ScopedCipher");
            Ok(cipher)
        }
        Err(err) => {
            debug!("Error initializing ZeroKMS ScopedCipher: {:?}", err);
            Err(err.into())
        }
    }
}

fn to_eql_encrypted(
    encrypted: Encrypted,
    identifier: &Identifier,
) -> Result<eql::Ciphertext, Error> {
    match encrypted {
        Encrypted::Record(ciphertext, _terms) => {
            let ct = eql::Ciphertext::new(ciphertext, identifier.to_owned());
            Ok(ct)
        }
        Encrypted::SteVec(_ste_vec_index) => {
            todo!("Encrypted::SteVec");
        }
    }
}
