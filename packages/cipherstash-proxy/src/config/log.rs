use std::{fmt::Display, io::IsTerminal};

use clap::ValueEnum;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct LogConfig {
    #[serde(default = "LogConfig::default_ansi_enabled")]
    pub ansi_enabled: bool,

    #[serde(default = "LogConfig::default_log_format")]
    pub format: LogFormat,

    #[serde(default = "LogConfig::default_log_output")]
    pub output: LogOutput,

    #[serde(default = "LogConfig::default_log_level")]
    pub level: LogLevel,

    // Log target level fields - generated from define_log_targets! macro
    #[serde(default = "LogConfig::default_log_level")]
    pub development_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub authentication_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub config_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub context_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub encoding_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub encrypt_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub decrypt_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub encrypt_config_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub keyset_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub mapper_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub migrate_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub protocol_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub proxy_level: LogLevel,

    #[serde(default = "LogConfig::default_log_level")]
    pub schema_level: LogLevel,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, ValueEnum)]
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogOutput {
    // Serde does not seem to have a case insensitive option. alias is clunky, but better than custom de/serialisers
    #[serde(alias = "Stdout", alias = "stdout", alias = "STDOUT")]
    Stdout,
    #[serde(alias = "Stderr", alias = "stderr", alias = "STDERR")]
    Stderr,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, ValueEnum)]
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

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        };
        write!(f, "{s}")
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self::with_level(LogConfig::default_log_level())
    }
}

impl LogConfig {
    pub fn with_level(level: LogLevel) -> Self {
        LogConfig {
            format: LogConfig::default_log_format(),
            output: LogConfig::default_log_output(),
            ansi_enabled: LogConfig::default_ansi_enabled(),
            level,
            // Log target level field assignments - matches define_log_targets! macro
            development_level: level,
            authentication_level: level,
            config_level: level,
            context_level: level,
            encoding_level: level,
            encrypt_level: level,
            decrypt_level: level,
            encrypt_config_level: level,
            keyset_level: level,
            mapper_level: level,
            migrate_level: level,
            protocol_level: level,
            proxy_level: level,
            schema_level: level,
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

    pub const fn default_log_output() -> LogOutput {
        LogOutput::Stdout
    }

    pub const fn default_log_level() -> LogLevel {
        LogLevel::Info
    }
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::with_no_cs_vars;
    use crate::{
        config::{LogFormat, LogLevel, LogOutput},
        error::Error,
        TandemConfig,
    };

    #[test]
    fn log_config_is_almost_case_insensitive() {
        with_no_cs_vars(|| {
            temp_env::with_vars([("CS_LOG__LEVEL", Some("error"))], || {
                let config =
                    TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert_eq!(config.log.level, LogLevel::Error);
            });

            temp_env::with_vars([("CS_LOG__LEVEL", Some("WARN"))], || {
                let config =
                    TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert_eq!(config.log.level, LogLevel::Warn);
            });

            temp_env::with_vars([("CS_LOG__OUTPUT", Some("stderr"))], || {
                let config =
                    TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert_eq!(config.log.output, LogOutput::Stderr);
            });

            temp_env::with_vars([("CS_LOG__FORMAT", Some("Pretty"))], || {
                let config =
                    TandemConfig::build("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert_eq!(config.log.format, LogFormat::Pretty);
            });

            temp_env::with_vars([("CS_LOG__FORMAT", Some("dEbUG"))], || {
                let config = TandemConfig::build("tests/config/cipherstash-proxy-test.toml");

                assert!(config.is_err());
                assert!(matches!(config.unwrap_err(), Error::Config(_)));
            });
        });
    }
}
