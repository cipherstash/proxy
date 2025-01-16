use crate::error::{ConfigError, Error};
use crate::log::global_default_log_level;
use config::{Config, Environment};
use regex::Regex;
use rustls_pki_types::ServerName;
use serde::Deserialize;
use std::path::PathBuf;
use std::{fmt::Display, time::Duration};

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
    pub development: Option<DevelopmentConfig>,
    pub log: Option<LogConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "ServerConfig::default_host")]
    pub host: String,

    #[serde(default = "ServerConfig::default_port")]
    pub port: u16,

    #[serde(default)]
    pub require_tls: bool,
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

    #[serde(default = "DatabaseConfig::default_connection_timeout")]
    pub connection_timeout: u64,

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

#[derive(Clone, Debug, Deserialize)]
pub struct DevelopmentConfig {
    #[serde(default)]
    pub disable_mapping: bool,

    #[serde(default)]
    pub disable_database_tls: bool,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct LogConfig {
    #[serde(default = "global_default_log_level")]
    pub development_level: String,
    #[serde(default = "global_default_log_level")]
    pub authentication_level: String,
    #[serde(default = "global_default_log_level")]
    pub context_level: String,
    #[serde(default = "global_default_log_level")]
    pub keyset_level: String,
    #[serde(default = "global_default_log_level")]
    pub protocol_level: String,
    #[serde(default = "global_default_log_level")]
    pub mapper_level: String,
    #[serde(default = "global_default_log_level")]
    pub schema_level: String,
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
            println!("Configuration file was not found: {path}");
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
            .map_err(|err| match err {
                config::ConfigError::Message(ref s) => match s {
                    s if s.contains("UUID parsing failed") => ConfigError::InvalidDatasetId,
                    s if s.contains("missing field") => {
                        let mut name = extract_field_name(s).map_or("unknown".to_string(), |s| s);

                        if name == "name" {
                            name = "database.name".to_string();
                        }

                        ConfigError::MissingParameter { name }
                    }
                    _ => err.into(),
                },
                _ => err.into(),
            })?;

        Ok(config)
    }

    pub fn disable_database_tls(&self) -> bool {
        match &self.development {
            Some(dev) => dev.disable_database_tls,
            None => false,
        }
    }

    pub fn disable_mapping(&self) -> bool {
        match &self.development {
            Some(dev) => dev.disable_mapping,
            None => false,
        }
    }
}

fn extract_field_name(input: &str) -> Option<String> {
    let re = Regex::new(r"`(\w+)`").unwrap();
    re.captures(input)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            host: ServerConfig::default_host(),
            port: ServerConfig::default_port(),
            require_tls: false,
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

    // 5 minutes
    pub fn default_connection_timeout() -> u64 {
        1000 * 60 * 5
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

    pub fn connection_timeout(&self) -> Duration {
        Duration::from_millis(self.connection_timeout)
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
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{config::TandemConfig, error::Error};

    const CS_PREFIX: &str = "CS_TEST";

    #[test]
    fn test_database_as_url() {
        let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
        assert_eq!(
            config.database.to_socket_address(),
            "localhost:5532".to_string()
        );
    }

    #[test]
    fn test_dataset_as_uuid() {
        temp_env::with_vars_unset(["CS_ENCRYPT__DATASET_ID"], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert_eq!(
                config.encrypt.dataset_id,
                Some(Uuid::parse_str("484cd205-99e8-41ca-acfe-55a7e25a8ec2").unwrap())
            );

            let config = TandemConfig::build("tests/config/cipherstash-proxy-bad-dataset.toml");

            assert!(config.is_err());
            assert!(matches!(config.unwrap_err(), Error::Config(_)));
        });
    }
}
