use super::tls::TlsConfig;
use super::{
    DatabaseConfig, LogConfig, LogLevel, ServerConfig, CS_PREFIX, DEBUG_THREAD_STACK_SIZE,
    DEFAULT_CONFIG_FILE_PATH, DEFAULT_THREAD_STACK_SIZE,
};
use crate::config::LogFormat;
use crate::error::{ConfigError, Error};
use crate::Args;
use cipherstash_client::config::vars::{
    CS_CLIENT_ACCESS_KEY, CS_CLIENT_ID, CS_CLIENT_KEY, CS_DEFAULT_KEYSET_ID, CS_WORKSPACE_ID,
};
use config::{Config, Environment};
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;
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
    pub default_keyset_id: Option<Uuid>,
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

        // Env vars from `stash setup --proxy`
        let stash_setup_source = Environment::with_prefix(CS_PREFIX)
            .separator("__")
            .prefix_separator("_")
            .source(Some({
                let mut env = HashMap::new();

                if let Ok(value) = std::env::var(CS_CLIENT_ID) {
                    env.insert("CS_ENCRYPT__CLIENT_ID".into(), value);
                }

                if let Ok(value) = std::env::var(CS_CLIENT_KEY) {
                    env.insert("CS_ENCRYPT__CLIENT_KEY".into(), value);
                }

                if let Ok(value) = std::env::var(CS_DEFAULT_KEYSET_ID) {
                    env.insert("CS_ENCRYPT__DEFAULT_KEYSET_ID".into(), value);
                }

                if let Ok(value) = std::env::var(CS_WORKSPACE_ID) {
                    env.insert("CS_AUTH__WORKSPACE_ID".into(), value);
                }

                if let Ok(value) = std::env::var(CS_CLIENT_ACCESS_KEY) {
                    env.insert("CS_AUTH__CLIENT_ACCESS_KEY".into(), value);
                }

                env
            }));

        // Source order is important!
        let config = Config::builder()
            .add_source(config::File::with_name(path).required(false))
            .add_source(cs_env_source)
            .add_source(stash_setup_source)
            .build()?
            .try_deserialize()
            .map_err(|err| {
                // ConfigError is not helping here
                //  - does not carry the information in structured form
                //  - missing parameters are returned by at least two different errors, depending the source of the error
                // Easier to inspect the error message.
                match err.to_string() {
                    s if s.contains("UUID parsing failed") => ConfigError::InvalidDatasetId,
                    s if s.contains("missing field") => {
                        let (field, key) = extract_missing_field_and_key(&s);
                        match (field, key) {
                            (field, None) if field == "auth" => ConfigError::MissingAuthKey,
                            (field, None) if field == "encrypt" => ConfigError::MissingEncryptKey,
                            (field, None) if field == "database" => ConfigError::MissingDatabaseKey,
                            (field, None) => ConfigError::MissingField { field },
                            (field, Some(key)) => ConfigError::MissingFieldForKey { key, field },
                        }
                    }
                    s if s.contains("does not have variant constructor") => {
                        let (name, value) = extract_invalid_field(&s);
                        ConfigError::InvalidParameter { name, value }
                    }
                    _ => err.into(),
                }
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

static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());

///
/// Extracts a field name (if present) from a config::ConfigError string
/// This is called in `build` if a ConfigError message contains the string `missing field`
/// Expected string is in the forms:
///     "missing field `{field}}` for key `{key}`"
///     "missing field `{field}}`"
///
fn extract_missing_field_and_key(input: &str) -> (String, Option<String>) {
    let default = "unknown";
    let values = RE
        .find_iter(input)
        .map(|m| m.as_str().trim_matches('`'))
        .collect::<Vec<_>>();
    (
        values.first().map_or(default.to_owned(), |s| s.to_string()),
        values.get(1).map(|s| s.to_string()),
    )
}

///
/// Extracts a field name (if present) from a config::ConfigError string
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
    use crate::{
        config::{tandem::extract_missing_field_and_key, TandemConfig},
        error::Error,
    };
    use cipherstash_client::config::vars::{
        CS_CLIENT_ACCESS_KEY, CS_CLIENT_ID, CS_CLIENT_KEY, CS_DEFAULT_KEYSET_ID, CS_WORKSPACE_ID,
    };
    use std::collections::HashMap;
    use uuid::Uuid;

    const CS_PREFIX: &str = "CS_TEST";

    #[test]
    /// the env vars from stash setup should be the preferred option
    /// File -> extended env (generated by the config struct layout) -> stash setup env
    fn with_stash_cli_config() {
        temp_env::with_vars(
            [
                // Orignal recipe ENV var
                ("CS_ENCRYPT__CLIENT_ID", Some("CS_ENCRYPT__CLIENT_ID")),
                (CS_CLIENT_ID, Some("CS_CLIENT_ID")),
                (CS_CLIENT_KEY, Some("CS_CLIENT_KEY")),
                (
                    CS_DEFAULT_KEYSET_ID,
                    Some("dd0a239f-02e2-4c8e-ba20-d9f0f85af9ac"),
                ),
                (CS_WORKSPACE_ID, Some("CS_WORKSPACE_ID")),
                (CS_CLIENT_ACCESS_KEY, Some("CS_CLIENT_ACCESS_KEY")),
            ],
            || {
                let config =
                    TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();

                assert_eq!(config.encrypt.client_id, "CS_CLIENT_ID".to_string());

                assert_eq!(
                    config.auth.client_access_key,
                    "CS_CLIENT_ACCESS_KEY".to_string()
                );

                assert_eq!(
                    config.encrypt.default_keyset_id,
                    Some(Uuid::parse_str("dd0a239f-02e2-4c8e-ba20-d9f0f85af9ac").unwrap())
                );
            },
        );
    }

    #[test]

    fn with_extended_env_naming() {
        temp_env::with_vars(
            [
                // Orignal recipe ENV var
                (
                    "CS_ENCRYPT__CLIENT_ID",
                    Some("dd0a239f-02e2-4c8e-ba20-d9f0f85af9ac"),
                ),
                (CS_CLIENT_ID, None),
            ],
            || {
                let config =
                    TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();

                assert_eq!(
                    &config.encrypt.client_id,
                    "dd0a239f-02e2-4c8e-ba20-d9f0f85af9ac"
                );
            },
        );
    }

    #[test]
    fn database_as_url() {
        let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
        assert_eq!(
            config.database.to_socket_address(),
            "localhost:5532".to_string()
        );
    }

    #[test]
    fn dataset_as_uuid() {
        temp_env::with_vars_unset(["CS_ENCRYPT__DATASET_ID", "CS_DEFAULT_KEYSET_ID"], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert_eq!(
                config.encrypt.default_keyset_id,
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

    #[test]
    /// the env vars from stash setup should be the preferred option
    /// File -> extended env (generated by the config struct layout) -> stash setup env
    fn extract_field_and_key_name_from_config_error() {
        let s = "missing field `client_access_key` for key `auth`";

        let (field, key) = extract_missing_field_and_key(s);

        assert_eq!(field, "client_access_key".to_string());
        assert_eq!(key.unwrap(), "auth".to_string());

        // Bad input - auth is extracted as field
        let s = "blah {client_access_key} for vtha `auth`";

        let (field, key) = extract_missing_field_and_key(s);

        assert_eq!(field, "auth".to_string());
        assert!(key.is_none());

        // Bad input - no field can be extracted is extracted as field
        let s = "blah {client_access_key} for vtha auth";

        let (field, key) = extract_missing_field_and_key(s);

        assert_eq!(field, "unknown".to_string());
        assert!(key.is_none());
    }

    /// Returns the default environment variables as a Vec
    fn default_env_vars() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("CS_CLIENT_ID", Some("00000000-0000-0000-0000-000000000000")),
            ("CS_CLIENT_KEY", Some("CS_CLIENT_KEY")),
            (
                "CS_DEFAULT_KEYSET_ID",
                Some("00000000-0000-0000-0000-000000000000"),
            ),
            ("CS_WORKSPACE_ID", Some("CS_WORKSPACE_ID")),
            ("CS_CLIENT_ACCESS_KEY", Some("CS_CLIENT_ACCESS_KEY")),
            ("CS_DATABASE__USERNAME", Some("CS_DATABASE__USERNAME")),
            ("CS_DATABASE__PASSWORD", Some("CS_DATABASE__PASSWORD")),
            ("CS_DATABASE__NAME", Some("CS_DATABASE__NAME")),
        ]
    }

    /// Merges the default environment variables with overrides
    fn merge_env_vars(
        overrides: Vec<(&'static str, Option<&'static str>)>,
    ) -> Vec<(&'static str, Option<&'static str>)> {
        let mut env_map: HashMap<&str, Option<&str>> = default_env_vars().into_iter().collect();

        for (key, value) in overrides {
            env_map.insert(key, value);
        }

        env_map.into_iter().collect()
    }

    #[test]
    fn missing_auth_config() {
        // Missing both workspace id and client_access_key

        let env = merge_env_vars(vec![
            ("CS_WORKSPACE_ID", None),
            ("CS_CLIENT_ACCESS_KEY", None),
        ]);

        temp_env::with_vars(env, || {
            let result = TandemConfig::build("tests/config/unknown.toml");
            assert!(result.is_err());

            if let Err(err) = result {
                assert!(err.to_string().contains("Missing [auth] configuration"));
            } else {
                unreachable!();
            }
        });

        // Missing client_access_key
        let env = merge_env_vars(vec![("CS_CLIENT_ACCESS_KEY", None)]);

        temp_env::with_vars(env, || {
            let result = TandemConfig::build("tests/config/unknown.toml");
            assert!(result.is_err());

            if let Err(err) = result {
                assert!(err
                    .to_string()
                    .contains("Missing client_access_key from [auth] configuration."));
            } else {
                unreachable!();
            }
        });
    }

    #[test]
    fn missing_encrypt_config() {
        // Missing all encrypt config
        let env = merge_env_vars(vec![
            ("CS_CLIENT_ID", None),
            ("CS_CLIENT_KEY", None),
            ("CS_DEFAULT_KEYSET_ID", None),
        ]);

        temp_env::with_vars(env, || {
            let result = TandemConfig::build("tests/config/unknown.toml");
            assert!(result.is_err());

            if let Err(err) = result {
                assert!(err.to_string().contains("Missing [encrypt] configuration"));
            } else {
                unreachable!();
            }
        });

        // Missing client_id
        let env = merge_env_vars(vec![("CS_CLIENT_ID", None)]);

        temp_env::with_vars(env, || {
            let result = TandemConfig::build("tests/config/unknown.toml");
            assert!(result.is_err());

            if let Err(err) = result {
                assert!(err
                    .to_string()
                    .contains("Missing client_id from [encrypt] configuration."));
            } else {
                unreachable!();
            }
        });
    }

    #[test]
    fn missing_database_config() {
        // Missing all database config

        let env = merge_env_vars(vec![
            ("CS_DATABASE__USERNAME", None),
            ("CS_DATABASE__PASSWORD", None),
            ("CS_DATABASE__NAME", None),
            ("CS_DATABASE__HOST", None),
            ("CS_DATABASE__PORT", None),
        ]);

        temp_env::with_vars(env, || {
            let result = TandemConfig::build("tests/config/unknown.toml");
            assert!(result.is_err());

            if let Err(err) = result {
                assert!(err.to_string().contains("Missing [database] configuration"));
            } else {
                unreachable!();
            }
        });

        // Missing name
        let env = merge_env_vars(vec![("CS_DATABASE__NAME", None)]);

        temp_env::with_vars(env, || {
            let result = TandemConfig::build("tests/config/unknown.toml");
            assert!(result.is_err());

            if let Err(err) = result {
                assert!(err
                    .to_string()
                    .contains("Missing name from [database] configuration."));
            } else {
                unreachable!();
            }
        });
    }
}
