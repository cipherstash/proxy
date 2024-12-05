use crate::{
    config::{DatasetManager, SchemaManager, TandemConfig},
    eql,
    error::{EncryptError, Error},
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

#[derive(Debug, Clone)]
pub struct Encrypt {
    pub config: TandemConfig,
    cipher: Arc<ScopedCipher>,
    dataset: DatasetManager,
    schema: SchemaManager,
}

impl Encrypt {
    pub async fn init(config: TandemConfig) -> Result<Encrypt, Error> {
        let cipher = Arc::new(init_cipher(&config).await?);

        let dataset = DatasetManager::init(&config.database).await?;
        let schema = SchemaManager::init(&config.database).await?;

        Ok(Encrypt {
            config,
            cipher,
            dataset,
            schema,
        })
    }

    pub async fn encrypt(
        &self,
        plaintexts: Vec<Option<eql::Plaintext>>,
    ) -> Result<Vec<Option<eql::Ciphertext>>, Error> {
        let mut pipeline = ReferencedPendingPipeline::new(self.cipher.clone());

        for (idx, pt) in plaintexts.iter().enumerate() {
            match pt {
                Some(pt) => {
                    let column_config = self.column_config(&pt)?;

                    let pt = Plaintext::Utf8Str(Some(pt.plaintext.to_owned()));
                    let encryptable = PlaintextTarget::new(pt, column_config.clone(), None);
                    pipeline.add_with_ref::<PlaintextTarget>(encryptable, idx)?;
                }
                None => {}
            }
        }

        let mut encrypted_eql = vec![];
        if !pipeline.is_empty() {
            let mut result = pipeline.encrypt().await?;

            for (idx, pt) in plaintexts.iter().enumerate() {
                match pt {
                    Some(pt) => {
                        let maybe_encrypted = result.remove(idx);
                        match maybe_encrypted {
                            Some(encrypted) => {
                                let ct = to_eql_encrypted(encrypted, pt)?;
                                encrypted_eql.push(Some(ct));
                            }
                            None => {
                                return Err(EncryptError::ColumnNotEncrypted {
                                    table: pt.identifier.table.to_owned(),
                                    column: pt.identifier.column.to_owned(),
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

    pub fn decrypt(&self, pt: Vec<eql::Ciphertext>) -> Result<Vec<eql::Plaintext>, Error> {
        Ok(vec![])
    }

    fn column_config(&self, pt: &eql::Plaintext) -> Result<ColumnConfig, Error> {
        let dataset = self.dataset.load();

        // TODO dataset config is inconsistent with input param types for get_table and get_column
        let table_config = dataset
            .get_table(&pt.identifier.table.as_str())
            .ok_or_else(|| EncryptError::UnknownTable {
                table: pt.identifier.table.to_owned(),
            })?;

        let column_config = table_config
            .get_column(&pt.identifier.column)?
            .ok_or_else(|| EncryptError::UnknownColumn {
                table: pt.identifier.table.to_owned(),
                column: pt.identifier.column.to_owned(),
            })?;

        Ok(column_config.clone())
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

fn to_eql_encrypted(encrypted: Encrypted, pt: &eql::Plaintext) -> Result<eql::Ciphertext, Error> {
    struct Indexes {
        ore_index: Option<String>,
        match_index: Option<Vec<u16>>,
        unique_index: Option<String>,
    }

    // TODO INDEXES
    // let mut indexes = Indexes {
    //     ore_index: None,
    //     match_index: None,
    //     unique_index: None,
    // };

    match encrypted {
        Encrypted::Record(ciphertext, _terms) => {
            let ct = eql::Ciphertext::new(ciphertext, pt.identifier.clone());
            Ok(ct)
        }
        Encrypted::SteVec(_ste_vec_index) => {
            todo!("Encrypted::SteVec");
        }
    }
}
