#[allow(clippy::module_inception)]
mod zerokms;

pub use zerokms::ZeroKms;

use crate::config::TandemConfig;
use crate::error::{Error, ZeroKMSError};
use crate::log::ZEROKMS;
use cipherstash_client::{
    zerokms::{ClientKey, ZeroKMSBuilder},
    AutoStrategy, ZeroKMS,
};

pub type ScopedCipher = cipherstash_client::encryption::ScopedCipher<AutoStrategy>;

pub type ZerokmsClient = ZeroKMS<AutoStrategy, ClientKey>;

pub(crate) fn init_zerokms_client(config: &TandemConfig) -> Result<ZerokmsClient, Error> {
    let strategy = AutoStrategy::builder()
        .with_access_key(&config.auth.client_access_key)
        .with_workspace_crn(config.auth.workspace_crn.clone())
        .detect()
        .map_err(|e| {
            tracing::warn!(target: ZEROKMS, msg = "ZeroKMS authentication strategy detection failed", error = %e);
            ZeroKMSError::AuthenticationFailed
        })?;

    let client_key = config.encrypt.build_client_key()?;

    let builder = ZeroKMSBuilder::new(strategy);
    Ok(builder.with_client_key(client_key).build()?)
}
