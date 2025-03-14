use super::tls::TlsConfig;
use super::{
    DatabaseConfig, LogConfig, LogLevel, ServerConfig, CS_PREFIX, DEBUG_THREAD_STACK_SIZE,
    DEFAULT_CONFIG_FILE_PATH, DEFAULT_THREAD_STACK_SIZE,
};
use crate::config::LogFormat;
use crate::error::{ConfigError, Error};
use crate::Args;
use config::{Config, Environment};
use regex::Regex;
use serde::Deserialize;
use std::env;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
pub struct TandemConfig {
    #[serde(default)]
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub encrypt: EncryptConfig,
    pub tls: Option<TlsConfig>,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub prometheus: PrometheusConfig,
    pub development: Option<DevelopmentConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthConfig {
    pub workspace_id: String,
    pub client_access_key: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct EncryptConfig {
    pub client_id: String,
    pub client_key: String,
    pub dataset_id: Option<Uuid>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PrometheusConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "PrometheusConfig::default_port")]
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DevelopmentConfig {
    #[serde(default)]
    pub disable_mapping: bool,

    #[serde(default)]
    pub disable_database_tls: bool,

    #[serde(default)]
    pub enable_mapping_errors: bool,
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

    pub fn load(args: &Args) -> Result<TandemConfig, Error> {
        // Log a warning to user that config file is missing
        if !PathBuf::from(&args.config_file_path).exists() {
            println!(
                "Configuration file was not found: {}",
                args.config_file_path
            );
            println!("Loading config values from environment variables.");
        }
        let mut config = TandemConfig::build(&args.config_file_path)?;

        // If log level is default, it has not been set by the user in config
        if config.log.level == LogConfig::default_log_level() {
            config.log.level = args.log_level;
        }

        // If log format is default, it has not been set by the user in config
        if config.log.format == LogConfig::default_log_format() {
            config.log.format = args.log_format;
        }

        Ok(config)
    }

    pub fn build(path: &str) -> Result<Self, Error> {
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
                    s if s.contains("does not have variant constructor") => {
                        let (name, value) = extract_invalid_field(s);
                        ConfigError::InvalidParameter { name, value }
                    }
                    _ => err.into(),
                },
                _ => err.into(),
            })?;

        Ok(config)
    }

    pub fn database_tls_disabled(&self) -> bool {
        match &self.development {
            Some(dev) => dev.disable_database_tls,
            None => false,
        }
    }

    pub fn mapping_disabled(&self) -> bool {
        match &self.development {
            Some(dev) => dev.disable_mapping,
            None => false,
        }
    }

    pub fn mapping_errors_enabled(&self) -> bool {
        match &self.development {
            Some(dev) => dev.enable_mapping_errors,
            None => false,
        }
    }

    pub fn use_structured_logging(&self) -> bool {
        matches!(self.log.format, LogFormat::Structured)
    }

    ///
    /// Returns true if Prometheus export is enabled
    ///
    pub fn prometheus_enabled(&self) -> bool {
        self.prometheus.enabled
    }

    ///
    /// Thread stack size
    /// Not defined using a default, as we depend on the log level to increase the size for debugging
    ///
    /// In order of precedence
    ///     config if explicitly set
    ///     RUST_MIN_STACK env var if set
    ///     DEBUG_THREAD_STACK_SIZE if log level is Debug or Trace
    ///     otherwise set to DEFAULT_THREAD_STACK_SIZE (2MiB)
    ///
    pub fn thread_stack_size(&self) -> usize {
        // If the config variable is set, use that value
        if let Some(thread_stack_size) = self.server.thread_stack_size {
            return thread_stack_size;
        }

        // If the environment variable is set, use that value
        if let Ok(stack_size) = env::var("RUST_MIN_STACK") {
            stack_size
                .parse()
                .inspect_err(|err| {
                    println!("Could not parse env var RUST_MIN_STACK: {}", err);
                    println!("Using the default thread stack size");
                })
                .unwrap_or(DEFAULT_THREAD_STACK_SIZE);
        }

        if self.log.level == LogLevel::Debug || self.log.level == LogLevel::Trace {
            return DEBUG_THREAD_STACK_SIZE;
        }

        DEFAULT_THREAD_STACK_SIZE
    }
}

impl PrometheusConfig {
    pub fn default_port() -> u16 {
        9930
    }
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        PrometheusConfig {
            enabled: false,
            port: PrometheusConfig::default_port(),
        }
    }
}

///
/// Extracts a field name (if present) from a config::ConfigError::Message
/// This is called in `build` if a ConfigError message contains the string `missing field`
///
fn extract_field_name(input: &str) -> Option<String> {
    let re = Regex::new(r"`(\w+)`").unwrap();
    re.captures(input)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

///
/// Extracts a field name (if present) from a config::ConfigError::Message
/// This is called in `build` if a ConfigError message contains the string `does not have variant constructor`
///
/// Error string is `enum {name} does not have variant constructor {value}`
///
fn extract_invalid_field(input: &str) -> (String, String) {
    let words = input.split(" ").collect::<Vec<_>>();

    let default_name = "unknown".to_string();
    let default_val = "".to_string();

    if !input.starts_with("enum") {
        return (default_name, default_val);
    }

    let name = words
        .get(1)
        .map_or(default_name.to_owned(), |w| w.to_string());

    let value = words
        .last()
        .map_or(default_val.to_owned(), |w| w.to_string());

    (name, value)
}

#[cfg(test)]
mod tests {
    use crate::{config::TandemConfig, error::Error};
    use uuid::Uuid;

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

    #[test]
    fn prometheus_config() {
        let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
        assert!(!config.prometheus_enabled());

        temp_env::with_vars([("CS_PROMETHEUS__ENABLED", Some("true"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert!(config.prometheus_enabled());
            assert!(config.prometheus.enabled);
            assert_eq!(config.prometheus.port, 9930);
        });

        temp_env::with_vars([("CS_PROMETHEUS__PORT", Some("7777"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert!(!config.prometheus_enabled());
            assert!(!config.prometheus.enabled);
            assert_eq!(config.prometheus.port, 7777);
        });

        temp_env::with_vars(
            [
                ("CS_PROMETHEUS__ENABLED", Some("true")),
                ("CS_PROMETHEUS__PORT", Some("7777")),
            ],
            || {
                let config =
                    TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert!(config.prometheus_enabled());
                assert!(config.prometheus.enabled);
                assert_eq!(config.prometheus.port, 7777);
            },
        );
    }
}
