use super::protected_string_deserializer;
use crate::error::{ConfigError, Error};
use rustls_pki_types::ServerName;
use serde::Deserialize;
use std::{fmt::Display, time::Duration};
use vitaminc_protected::{Controlled, Protected};

#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "DatabaseConfig::default_host")]
    pub host: String,

    #[serde(default = "DatabaseConfig::default_port")]
    pub port: u16,

    pub name: String,

    #[serde(default = "DatabaseConfig::default_username")]
    pub username: String,

    #[serde(deserialize_with = "protected_string_deserializer")]
    password: Protected<String>,

    pub connection_timeout: Option<u64>,

    #[serde(default)]
    pub with_tls_verification: bool,

    #[serde(default = "DatabaseConfig::default_config_reload_interval")]
    pub config_reload_interval: u64,

    #[serde(default = "DatabaseConfig::default_schema_reload_interval")]
    pub schema_reload_interval: u64,
}

impl DatabaseConfig {
    pub fn default_host() -> String {
        "127.0.0.1".to_string()
    }

    pub const fn default_port() -> u16 {
        5432
    }

    pub fn default_username() -> String {
        "postgres".to_string()
    }

    pub const fn default_config_reload_interval() -> u64 {
        60
    }

    pub const fn default_schema_reload_interval() -> u64 {
        60
    }

    pub fn to_socket_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn to_connection_config(&self) -> tokio_postgres::Config {
        let mut db_config = tokio_postgres::Config::new();
        let password = self.password();
        db_config
            .host(&self.host)
            .port(self.port)
            .user(&self.username)
            .password(password)
            .dbname(&self.name);

        db_config
    }

    pub fn password(&self) -> String {
        self.password.to_owned().risky_unwrap()
    }

    const DEFAULT_CONNECTION_TIMEOUT_MS: u64 = 120_000;

    pub fn connection_timeout(&self) -> Option<Duration> {
        match self.connection_timeout {
            Some(0) => None,
            Some(ms) => Some(Duration::from_millis(ms)),
            None => Some(Duration::from_millis(Self::DEFAULT_CONNECTION_TIMEOUT_MS)),
        }
    }

    pub fn server_name(&self) -> Result<ServerName<'_>, Error> {
        let name = ServerName::try_from(self.host.as_str()).map_err(|_| {
            ConfigError::InvalidServerName {
                name: self.host.to_owned(),
            }
        })?;
        Ok(name)
    }

    #[cfg(test)]
    pub fn for_testing() -> Self {
        Self {
            host: Self::default_host(),
            port: Self::default_port(),
            name: "test".to_string(),
            username: "test".to_string(),
            password: Protected::new("test".to_string()),
            connection_timeout: None,
            with_tls_verification: false,
            config_reload_interval: Self::default_config_reload_interval(),
            schema_reload_interval: Self::default_schema_reload_interval(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_timeout_defaults_to_120_seconds() {
        let config = DatabaseConfig::for_testing();
        assert_eq!(
            config.connection_timeout(),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn connection_timeout_zero_disables_timeout() {
        let mut config = DatabaseConfig::for_testing();
        config.connection_timeout = Some(0);
        assert_eq!(config.connection_timeout(), None);
    }

    #[test]
    fn connection_timeout_custom_value_in_millis() {
        let mut config = DatabaseConfig::for_testing();
        config.connection_timeout = Some(5000);
        assert_eq!(
            config.connection_timeout(),
            Some(Duration::from_millis(5000))
        );
    }
}

///
/// Password is NEVER EVER displayed
///
impl Display for DatabaseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}@{}:{}/{}",
            self.username, self.host, self.port, self.name,
        )
    }
}
