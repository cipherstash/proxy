#[allow(clippy::module_inception)]
mod zerokms;

pub use zerokms::ZeroKms;

use crate::config::TandemConfig;
use cipherstash_client::config::{ConfigError, ZeroKMSConfigWithClientKey};
use cipherstash_client::{
    config::EnvSource,
    credentials::{auto_refresh::AutoRefresh, ServiceCredentials},
    zerokms::ClientKey,
    ConsoleConfig, CtsConfig, ZeroKMS, ZeroKMSConfig,
};

pub type ScopedCipher =
    cipherstash_client::encryption::ScopedCipher<AutoRefresh<ServiceCredentials>>;

pub type ZerokmsClient = ZeroKMS<AutoRefresh<ServiceCredentials>, ClientKey>;

pub(crate) fn init_zerokms_client(
    config: &TandemConfig,
) -> Result<ZeroKMS<AutoRefresh<ServiceCredentials>, ClientKey>, ConfigError> {
    let zerokms_config = build_zerokms_config(config)?;

    Ok(zerokms_config
        .create_client_with_credentials(AutoRefresh::new(zerokms_config.credentials())))
}

pub fn build_zerokms_config(
    config: &TandemConfig,
) -> Result<ZeroKMSConfigWithClientKey, ConfigError> {
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
