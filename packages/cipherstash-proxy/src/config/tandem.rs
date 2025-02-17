use crate::error::{ConfigError, Error};
use crate::log::CONFIG;
use config::{Config, Environment};
use regex::Regex;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use serde::Deserialize;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::{fmt::Display, time::Duration};
use tracing::debug;
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
    #[serde(default)]
    pub log: LogConfig,
    pub prometheus: PrometheusConfig,
    pub development: Option<DevelopmentConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "ServerConfig::default_host")]
    pub host: String,

    #[serde(default = "ServerConfig::default_port")]
    pub port: u16,

    #[serde(default)]
    pub require_tls: bool,

    #[serde(default = "ServerConfig::default_shutdown_timeout")]
    pub shutdown_timeout: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PrometheusConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "PrometheusConfig::default_port")]
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
#[serde(tag = "type")]
pub enum TlsConfig {
    Pem {
        certificate: String,
        private_key: String,
    },
    Path {
        certificate: String,
        private_key: String,
    },
}

impl TlsConfig {
    pub fn certificate(&self) -> &str {
        match self {
            Self::Pem { certificate, .. } | Self::Path { certificate, .. } => certificate,
        }
    }

    pub fn private_key(&self) -> &str {
        match self {
            Self::Pem { private_key, .. } | Self::Path { private_key, .. } => private_key,
        }
    }

    pub fn certificate_err_msg(&self) -> &str {
        match self {
            Self::Pem { .. } => "Transport Layer Security (TLS) Certificate is invalid",
            Self::Path { .. } => "Transport Layer Security (TLS) Certificate not found",
        }
    }

    pub fn private_key_err_msg(&self) -> &str {
        match self {
            Self::Pem { .. } => "Transport Layer Security (TLS) Private key is invalid",
            Self::Path { .. } => "Transport Layer Security (TLS) Private key not found",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct KeyCertPair {
    pub certificate: String,
    pub private_key: String,
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
pub struct DevelopmentConfig {
    #[serde(default)]
    pub disable_mapping: bool,

    #[serde(default)]
    pub disable_database_tls: bool,

    #[serde(default)]
    pub enable_mapping_errors: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LogConfig {
    #[serde(default = "LogConfig::default_ansi_enabled")]
    pub ansi_enabled: bool,

    #[serde(default = "LogConfig::default_log_format")]
    pub format: LogFormat,

    #[serde(default = "LogConfig::default_log_output")]
    pub output: LogOutput,

    #[serde(default = "LogConfig::default_log_level")]
    pub level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub development_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub authentication_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub context_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub encrypt_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub keyset_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub protocol_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub mapper_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub schema_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub config_level: LogLevel,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    // Serde does not seem to have a case insensitive option. alias is clunky, but better than custom de/serialisers
    #[serde(alias = "Pretty", alias = "pretty", alias = "PRETTY")]
    Pretty,
    #[serde(alias = "Structured", alias = "structured", alias = "STRUCTURED")]
    Structured,
    #[serde(alias = "Text", alias = "text", alias = "TEXT")]
    Text,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogOutput {
    // Serde does not seem to have a case insensitive option. alias is clunky, but better than custom de/serialisers
    #[serde(alias = "Stdout", alias = "stdout", alias = "STDOUT")]
    Stdout,
    #[serde(alias = "Stderr", alias = "stderr", alias = "STDERR")]
    Stderr,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    // Serde does not seem to have a case insensitive option. alias is clunky, but better than custom de/serialisers
    #[serde(alias = "Error", alias = "error", alias = "ERROR")]
    Error,
    #[serde(alias = "Warn", alias = "warn", alias = "WARN")]
    Warn,
    #[serde(alias = "Info", alias = "info", alias = "INFO")]
    Info,
    #[serde(alias = "Debug", alias = "debug", alias = "DEBUG")]
    Debug,
    #[serde(alias = "Trace", alias = "trace", alias = "TRACE")]
    Trace,
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

    ///
    /// Prometheus is enabled if
    ///  - enabled is true
    ///  - a port has been explicitly set
    ///
    pub fn prometheus_enabled(&self) -> bool {
        self.prometheus.enabled || self.prometheus.port != PrometheusConfig::default_port()
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

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            host: ServerConfig::default_host(),
            port: ServerConfig::default_port(),
            require_tls: false,
            shutdown_timeout: ServerConfig::default_shutdown_timeout(),
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

    pub fn default_shutdown_timeout() -> u64 {
        2000
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

    pub fn shutdown_timeout(&self) -> Duration {
        Duration::from_millis(self.shutdown_timeout)
    }
}

impl PrometheusConfig {
    pub fn default_port() -> u16 {
        9930
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
        match self {
            TlsConfig::Pem { certificate, .. } => {
                debug!(target: CONFIG, msg = "TLS certificate is a pem string (content omitted)");
                let certs = CertificateDer::pem_slice_iter(certificate.as_bytes())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap_or(Vec::new());
                !certs.is_empty()
            }
            TlsConfig::Path { certificate, .. } => {
                debug!(target: CONFIG, msg = "TLS certificate is a path: {}", certificate);
                PathBuf::from(certificate).exists()
            }
        }
    }

    pub fn private_key_exists(&self) -> bool {
        match self {
            TlsConfig::Pem { private_key, .. } => {
                debug!(target: CONFIG, msg = "TLS private_key is a pem string (content omitted)");
                PrivateKeyDer::from_pem_slice(private_key.as_bytes()).is_ok()
            }
            TlsConfig::Path { private_key, .. } => {
                debug!(target: CONFIG, msg = "TLS private_key is a path: {}", private_key);
                PathBuf::from(private_key).exists()
            }
        }
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        };
        write!(f, "{}", s)
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            format: LogConfig::default_log_format(),
            output: LogConfig::default_log_output(),
            ansi_enabled: LogConfig::default_ansi_enabled(),
            level: LogConfig::default_log_level(),
            development_level: LogConfig::default_log_level(),
            authentication_level: LogConfig::default_log_level(),
            context_level: LogConfig::default_log_level(),
            encrypt_level: LogConfig::default_log_level(),
            keyset_level: LogConfig::default_log_level(),
            protocol_level: LogConfig::default_log_level(),
            mapper_level: LogConfig::default_log_level(),
            schema_level: LogConfig::default_log_level(),
            config_level: LogConfig::default_log_level(),
        }
    }
}

impl LogConfig {
    pub fn with_level(level: LogLevel) -> Self {
        LogConfig {
            format: LogConfig::default_log_format(),
            output: LogConfig::default_log_output(),
            ansi_enabled: LogConfig::default_ansi_enabled(),
            level,
            development_level: level,
            authentication_level: level,
            context_level: level,
            encrypt_level: level,
            keyset_level: level,
            protocol_level: level,
            mapper_level: level,
            schema_level: level,
            config_level: level,
        }
    }

    pub fn default_log_format() -> LogFormat {
        if std::io::stdout().is_terminal() {
            LogFormat::Pretty
        } else {
            LogFormat::Structured
        }
    }

    pub fn default_ansi_enabled() -> bool {
        std::io::stdout().is_terminal()
    }

    pub fn default_log_output() -> LogOutput {
        LogOutput::Stdout
    }

    pub fn default_log_level() -> LogLevel {
        LogLevel::Info
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        config::{LogFormat, LogLevel, LogOutput, TandemConfig},
        error::Error,
    };

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
    fn log_config_is_almost_case_insensitive() {
        temp_env::with_vars([("CS_LOG__LEVEL", Some("error"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert_eq!(config.log.level, LogLevel::Error);
        });

        temp_env::with_vars([("CS_LOG__LEVEL", Some("WARN"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert_eq!(config.log.level, LogLevel::Warn);
        });

        temp_env::with_vars([("CS_LOG__OUTPUT", Some("stderr"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert_eq!(config.log.output, LogOutput::Stderr);
        });

        temp_env::with_vars([("CS_LOG__FORMAT", Some("Pretty"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert_eq!(config.log.format, LogFormat::Pretty);
        });

        temp_env::with_vars([("CS_LOG__FORMAT", Some("dEbUG"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml");

            assert!(config.is_err());
            assert!(matches!(config.unwrap_err(), Error::Config(_)));
        });
    }

    fn test_config_with_path() -> TlsConfig {
        TlsConfig::Path {
            certificate: "../../tests/tls/server.cert".to_string(),
            private_key: "../../tests/tls/server.key".to_string(),
        }
    }

    fn test_config_with_invalid_path() -> TlsConfig {
        TlsConfig::Path {
            certificate: "/path/to/non-existent/file".to_string(),
            private_key: "/path/to/non-existent/file".to_string(),
        }
    }

    fn test_config_with_pem() -> TlsConfig {
        TlsConfig::Pem {
            certificate: "\
-----BEGIN CERTIFICATE-----
MIIDKzCCAhOgAwIBAgIUMXfu7Mj22j+e9Gt2gjV73TBg20wwDQYJKoZIhvcNAQEL
BQAwFDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI1MDEyNjAxNDkzMVoXDTI2MDEy
NjAxNDkzMVowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEF
AAOCAQ8AMIIBCgKCAQEApuqOqv0P8IPe7TmQGO2HeO0DjPrIVyYYCtJXEyUhPSuq
20ePjb6PyCeAlG873fJW4+fSF6YP0nsb7PJQYYa7E5CxJNYtjJ9c19l0CpgkNmHP
ogK8HhAokvjxKGTwidj37KAVBXznaWPfFvf8SuS2xFSCknTou2m67Q68rCYM8h9e
AjB5L0kL2bM6ySgGt5m0lWsr73duaGrLEJxfjV+JVitDSqLqbeDWOKXHfaKBBwL1
BZWyl4f4MM0rhLoDcbGOYIlkZtadB2lqdaFqfuejIcmZd/Q41mRhNmwNnG9guSWC
YHMdPkIrAaZNZlL0drIeTVgPcVkP8lPEkXsxHhmybwIDAQABo3UwczAdBgNVHQ4E
FgQUWQ8oySVGv/BhOr1B6zMVyNDeobkwHwYDVR0jBBgwFoAUWQ8oySVGv/BhOr1B
6zMVyNDeobkwDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCBaAwEwYDVR0lBAww
CgYIKwYBBQUHAwEwDQYJKoZIhvcNAQELBQADggEBAFzLi09kyRBE/H3RarjQdolv
eAPwpn16YqUgppYjKIbPHx6QtXBElhhqTlW104x8CMzx3pT0wBIaUPmhWj6DeWET
TZIDbXhWiMRhsB7cup7y5O9mlXvST4fyrcD30rgfO8XAL8nJLsAbCgL/BWlptC1m
2tFtY1Y8bYTD04TMVVVA20rvwwINg1Gd+JYRoHysBvnGuespMVuW0Ji49U7OWPp/
Iwy49Eyr7U0xg2VFPNBkNUmw6MQQVumt3OBydAKmd3XAJy/Nmzq/ZHvL3jdl1jlC
TU/T2RF2sDsSHrUIVMeifhYc0jfNlRwnUG5liN9BiGo1QxNZ9jGY/3ts5eu8+XM=
-----END CERTIFICATE-----
"
            .to_string(),
            private_key: "\
-----BEGIN PRIVATE KEY-----
MIIEugIBADANBgkqhkiG9w0BAQEFAASCBKQwggSgAgEAAoIBAQCm6o6q/Q/wg97t
OZAY7Yd47QOM+shXJhgK0lcTJSE9K6rbR4+Nvo/IJ4CUbzvd8lbj59IXpg/Sexvs
8lBhhrsTkLEk1i2Mn1zX2XQKmCQ2Yc+iArweECiS+PEoZPCJ2PfsoBUFfOdpY98W
9/xK5LbEVIKSdOi7abrtDrysJgzyH14CMHkvSQvZszrJKAa3mbSVayvvd25oassQ
nF+NX4lWK0NKoupt4NY4pcd9ooEHAvUFlbKXh/gwzSuEugNxsY5giWRm1p0HaWp1
oWp+56MhyZl39DjWZGE2bA2cb2C5JYJgcx0+QisBpk1mUvR2sh5NWA9xWQ/yU8SR
ezEeGbJvAgMBAAECgf8E32YqIqznJ9ZwvCIg2FBdc1fHRFJ78Few64VugtCMwVu8
/fCsDTVzIOHR7dXlK5z7JY1VCURxInql5uwYsfIbcvfdtdt8GNL2tiNs7WHryZRP
CVgcnCkQ++Koy4RcjbI9FEgQPjPLFK8Hx8JDvG90nSfIVMkp34t3Hs4Hu8IRr5Cq
dv1PsYzoa2DJb/gsed7fefm7MQ2SGH0r9TrA+rzUx3Vb05z5Wi/AEsCReLaWbplJ
ARwQCcfvMOAA3CvDkLH2m1J64EqS/vt6fmiE9x8KOU9OZ0FK6pP8evvHpkyaopqN
59DcNzDvGVyxLtwJ6JoQXLsoZywHIjah+eGu6ikCgYEA1TT2Sgx2E+4NOefPvuMg
DkT/3EYnXEOufI+rrr01J84gn1IuukC4nfKxel5KgVhMxZHHmB25Kp8G9tdDgVMd
qHdT5oMZgYAW7+vtQOWf8Px7P80WvN38LlI/v2bngSPnNhrg3MsBzpqnXtOlBFfR
Zq3PhWkwzCnSvuSbLELszOsCgYEAyGsUjcFFyF/so9FA6rrNplwisUy3ykBO98Ye
KIa5Dz3UsGqYraqk59MIC5f1BdeYRlVKUNlxcmT089goc0MxwKbqJHJdTVqWrnnK
o5+jAddv/awbuMYbt+//zM296SyXgi8y6eUt6TN8ss4NztpcxzBNmCrny8s6Xd9K
OqX9P40CgYBhE4xQivv4dxtuki31LFUcKi6VjRu+1tJLxN7W4S+iwCf6YuEDzRRC
Vo6YuPYTjrDmBEps6Ju23FG/cqQ57i5C1pJNEsQ6Qqgu9a1BL0xz3YIAutDvjeOU
874y2BfwpPhRmktoPMbF24T5mEQ6hgHCTsF+bTbavvBGGrDMpmxLoQKBgFjsWeRD
esja9s4AjEMZuyEzBBmSpoFQYzlAaCUnEXkXwAS+Zxu2+Q/67DjopUiATgn20dBp
ihJthNmkcN4jVDHcXUrqi0dFCFJFq4lJzTOF+SSednZXP/kuvVqLdtW8eUTD2F06
2FH+DDfxgOLktAGVBvibINmlRDJeXjsDZwgJAoGAOL28xi4UqaFOu4CbB5BvCIxN
l0AUk9ZCx4hOwE7BUqG9winPtmwqoXGtMuamlKf7vxONhg68EHFyDuMxL8rgHjrH
eq8W0CchxrihmoEm6zGtDbrdJ6KkbhyeFJgZPKX8Nff7Nsi7FJyea53CCv3B5aQr
B+qwsnNEiDoJhgYj+cQ=
-----END PRIVATE KEY-----
"
            .to_string(),
        }
    }

    fn test_config_with_invalid_pem() -> TlsConfig {
        TlsConfig::Pem {
            certificate: "-----INVALID PEM-----".to_string(),
            private_key: "-----INVALID PEM-----".to_string(),
        }
    }

    #[test]
    fn test_tls_cert_exists_with_path() {
        assert!(test_config_with_path().cert_exists());
        assert!(!test_config_with_invalid_path().cert_exists());
    }

    #[test]
    fn test_tls_cert_exists_with_pem() {
        assert!(test_config_with_pem().cert_exists());
        assert!(!test_config_with_invalid_pem().cert_exists());
    }

    #[test]
    fn test_tls_private_key_exists_with_path() {
        assert!(test_config_with_path().private_key_exists());
        assert!(!test_config_with_invalid_path().private_key_exists());
    }

    #[test]
    fn test_tls_private_key_exists_with_pem() {
        assert!(test_config_with_pem().private_key_exists());
        assert!(!test_config_with_invalid_pem().private_key_exists());
    }

    #[test]
    fn prometheus_config() {
        let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
        assert!(!config.prometheus_enabled());

        temp_env::with_vars([("CS_PROMETHEUS__ENABLED", Some("true"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert!(config.prometheus_enabled());
            assert_eq!(config.prometheus.port, 9930);
        });

        temp_env::with_vars([("CS_PROMETHEUS__PORT", Some("7777"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert!(config.prometheus_enabled());
        });

        temp_env::with_vars([("CS_PROMETHEUS__PORT", Some("9930"))], || {
            let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert!(!config.prometheus_enabled());
        });
    }
}
