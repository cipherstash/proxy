use crate::error::{ConfigError, Error};
use config::{Config, Environment};
use rustls::ServerConfig as TlsServerConfig;
use rustls_pki_types::{pem::PemObject, CertificateDer};
use rustls_pki_types::{PrivateKeyDer, ServerName};
use serde::Deserialize;
use std::fmt::Display;
use std::path::PathBuf;
use tracing::{debug, error};

use uuid::Uuid;

use super::{CS_PREFIX, DEFAULT_CONFIG_FILE_PATH};

#[derive(Clone, Debug, Deserialize)]
pub struct TandemConfig {
    #[serde(default)]
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub encrypt: EncryptConfig,
    pub tls: Option<TlsConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "ServerConfig::default_host")]
    pub host: String,

    #[serde(default = "ServerConfig::default_port")]
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "DatabaseConfig::default_host")]
    pub host: String,

    #[serde(default = "DatabaseConfig::default_port")]
    pub port: u16,

    pub name: String,
    pub username: String,
    pub password: String,

    #[serde(default)]
    pub with_tls_verification: bool,

    #[serde(default = "DatabaseConfig::default_config_reload_interval")]
    pub config_reload_interval: u64,

    #[serde(default = "DatabaseConfig::default_schema_reload_interval")]
    pub schema_reload_interval: u64,
}

///
/// Server TLS Configuration
/// This is listener/inbound connection config
///
#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
    pub certificate: String,
    pub private_key: String,
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
        // Log a warning to user that config file is missing
        if !PathBuf::from(path).exists() {
            println!("Config file not found: {path}");
            println!("Loading config values from environment variables.");
        }
        TandemConfig::build(path)
    }

    fn build(path: &str) -> Result<Self, Error> {
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
                            error!("Invalid dataset id. The configured dataset id must be a valid UUID.");
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

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            host: ServerConfig::default_host(),
            port: ServerConfig::default_port(),
        }
    }
}

impl ServerConfig {
    pub fn default_host() -> String {
        "0.0.0.0".to_string()
    }

    pub fn default_port() -> u16 {
        6432
    }

    pub fn server_name(&self) -> Result<ServerName, Error> {
        let name = ServerName::try_from(self.host.as_str()).map_err(|_| {
            ConfigError::InvalidServerName {
                name: self.host.to_owned(),
            }
        })?;
        Ok(name)
    }

    pub fn to_socket_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl DatabaseConfig {
    pub fn default_host() -> String {
        "127.0.0.1".to_string()
    }

    pub fn default_port() -> u16 {
        5432
    }

    pub fn default_config_reload_interval() -> u64 {
        60
    }

    pub fn default_schema_reload_interval() -> u64 {
        60
    }

    pub fn to_socket_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn to_connection_string(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.username, self.password, self.host, self.port, self.name
        )
    }
}

impl Display for DatabaseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}@{}:{}/{}",
            self.username, self.host, self.port, self.name,
        )
    }
}

impl TlsConfig {
    pub fn cert_exists(&self) -> bool {
        PathBuf::from(&self.certificate).exists()
    }

    pub fn private_key_exists(&self) -> bool {
        PathBuf::from(&self.private_key).exists()
    }

    pub fn server_config(&self) -> Result<TlsServerConfig, Error> {
        let certs =
            CertificateDer::pem_file_iter(&self.certificate)?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(&self.private_key)?;

        let config = TlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{config::TandemConfig, error::Error, trace};

    #[test]
    fn test_database_as_url() {
        trace();

        let config = TandemConfig::load("tests/config/cipherstash-proxy.toml").unwrap();
        assert_eq!(
            config.database.to_socket_address(),
            "localhost:5432".to_string()
        );
    }

    #[test]
    fn test_dataset_as_uuid() {
        trace();

        let config = TandemConfig::load("tests/config/cipherstash-proxy.toml").unwrap();
        assert_eq!(
            config.encrypt.dataset_id,
            Some(Uuid::parse_str("484cd205-99e8-41ca-acfe-55a7e25a8ec2").unwrap())
        );

        let config = TandemConfig::load("tests/config/cipherstash-proxy-bad-dataset.toml");

        assert!(config.is_err());
        assert!(matches!(config.unwrap_err(), Error::Config(_)));
    }
}
