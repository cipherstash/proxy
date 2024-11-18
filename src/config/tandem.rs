use crate::error::Error;
use config::{Config, Environment};
use serde::Deserialize;
use std::path::PathBuf;
use tracing::{debug, error};
use url::Url;
use uuid::Uuid;

use super::{CS_PREFIX, DEFAULT_CONFIG_FILE_PATH};

#[derive(Clone, Debug, Deserialize)]
pub struct TandemConfig {
    pub connect: ConnectionConfig,
    pub auth: AuthConfig,
    pub encrypt: EncryptConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ConnectionConfig {
    pub database: Url,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthConfig {
    pub workspace_id: String,
    pub client_access_key: String,
}

// TODO: Use Paranoid from the primitives crate when that lands
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct EncryptConfig {
    pub client_id: String,
    pub client_key: String,
    pub dataset_id: Option<Uuid>,
}

/// Config defaults to a file called `tandem` in the current directory.
/// Supports TOML, JSON, YAML
/// Variable names should match the struct field names.
///
/// ENV vars can be used to override file settings.
///
/// ENV vars must be prefixed with `CS_`.
///
impl TandemConfig {
    pub fn default_path() -> String {
        DEFAULT_CONFIG_FILE_PATH.to_string()
    }

    pub fn load(path: &str) -> Result<TandemConfig, Error> {
        TandemConfig::build_with_path(path)
    }

    pub fn build_with_path(path: &str) -> Result<Self, Error> {
        // Log a warning to user that config file is missing
        if !PathBuf::from(path).exists() {
            println!("Config file not found: {path}");
            println!("Loading config values only from environment variables.");
        }

        // For parsing top-level values such as CS_HOST, CS_PORT
        // and for parsing nested env values such as CS_DATABASE__HOST, CS_DATABASE__PORT
        let cs_env_source = Environment::with_prefix(CS_PREFIX)
            .try_parsing(true)
            .separator("__")
            .prefix_separator("_");

        let config: Self = Config::builder()
            .add_source(config::File::with_name(path).required(false))
            .add_source(cs_env_source)
            .build()?
            .try_deserialize()
            .map_err(|err| {
                match err {
                    config::ConfigError::Message(ref s) => {
                        if s.contains("UUID parsing failed") {
                            error!("Invalid Dataset ID. Expected a UUID.");
                            debug!("{s}");
                        }
                    }
                    _ => {}
                };
                err
            })?;

        Ok(config)
    }
}

impl ConnectionConfig {
    pub fn to_socket_address(&self) -> String {
        let host = self.database.host_str().unwrap_or("localhost");
        let port = self.database.port().unwrap_or(5432);

        format!("{}:{}", host, port)
    }

    pub fn to_connection_string(&self) -> String {
        self.database.to_string()
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{config::TandemConfig, error::Error, trace};

    #[test]
    fn test_database_as_url() {
        trace();

        let config = TandemConfig::load("tests/cipherstash-proxy.toml").unwrap();
        assert_eq!(
            config.connect.to_socket_address(),
            "localhost:5432".to_string()
        );
    }

    #[test]
    fn test_dataset_as_uuid() {
        trace();

        let config = TandemConfig::load("tests/cipherstash-proxy.toml").unwrap();
        assert_eq!(
            config.encrypt.dataset_id,
            Some(Uuid::parse_str("484cd205-99e8-41ca-acfe-55a7e25a8ec2").unwrap())
        );

        let config = TandemConfig::load("tests/cipherstash-proxy-bad-dataset.toml");

        assert!(config.is_err());
        assert!(matches!(config.unwrap_err(), Error::Config(_)));
    }
}
