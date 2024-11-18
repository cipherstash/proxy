// mod config;

use crate::{config::TandemConfig, eql, error::Error};
use cipherstash_client::{
    credentials::{auto_refresh::AutoRefresh, service_credentials::ServiceCredentials},
    encryption::{self, PlaintextTarget, ReferencedPendingPipeline},
    ConsoleConfig, CtsConfig, ZeroKMS, ZeroKMSConfig,
};
use std::{sync::Arc, vec};

type ScopedCipher = encryption::ScopedCipher<AutoRefresh<ServiceCredentials>>;

#[derive(Debug)]
pub struct Encrypt {
    pub config: TandemConfig,
    cipher: Arc<ScopedCipher>,
}

impl Clone for Encrypt {
    fn clone(&self) -> Self {
        Encrypt {
            config: self.config.clone(),
            cipher: self.cipher.clone(),
        }
    }
}

impl Encrypt {
    pub async fn init(config: TandemConfig) -> Result<Encrypt, Error> {
        let cipher = Arc::new(init_cipher(&config).await?);
        Ok(Encrypt { config, cipher })
    }

    pub fn encrypt(
        &self,
        pt: Vec<Option<eql::Plaintext>>,
    ) -> Result<Vec<Option<eql::Encrypted>>, Error> {
        let mut pipeline = ReferencedPendingPipeline::new(self.cipher.clone());

        // let encryptable = PlaintextTarget::new(plaintext, column_config.clone(), None);
        // pipeline.add_with_ref::<PlaintextTarget>(encryptable, idx)?;

        Ok(vec![])
    }

    pub fn decrypt(&self, pt: Vec<eql::Encrypted>) -> Result<Vec<eql::Plaintext>, Error> {
        Ok(vec![])
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

    Ok(ScopedCipher::init(Arc::new(zerokms_client), config.encrypt.dataset_id).await?)
}
