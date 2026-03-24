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
use url::Url;

pub type ScopedCipher = cipherstash_client::encryption::ScopedCipher<AutoStrategy>;

pub type ZerokmsClient = ZeroKMS<AutoStrategy, ClientKey>;

pub(crate) fn init_zerokms_client(config: &TandemConfig) -> Result<ZerokmsClient, Error> {
    if config.cts_host().is_some() {
        tracing::warn!(
            target: "config",
            "development.cts_host is configured but no longer supported. \
             CTS endpoint is now resolved automatically from credentials. \
             Remove development.cts_host from your configuration."
        );
    }

    let strategy = AutoStrategy::builder()
        .with_access_key(&config.auth.client_access_key)
        .with_workspace_crn(config.auth.workspace_crn.clone())
        .detect()
        .map_err(|e| {
            tracing::warn!(target: ZEROKMS, msg = "ZeroKMS authentication strategy detection failed", error = %e);
            ZeroKMSError::AuthenticationFailed
        })?;

    let client_key = config.encrypt.build_client_key()?;

    let mut builder = ZeroKMSBuilder::new(strategy);

    if let Some(zerokms_host) = config.zerokms_host() {
        let url = Url::parse(&zerokms_host).map_err(|_| {
            Error::from(crate::error::ConfigError::InvalidParameter {
                name: "development.zerokms_host".to_string(),
                value: zerokms_host,
            })
        })?;
        builder = builder.with_base_url(url);
    }

    Ok(builder.with_client_key(client_key).build()?)
}
