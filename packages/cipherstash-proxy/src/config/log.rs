use std::{fmt::Display, io::IsTerminal};

use clap::ValueEnum;
use serde::Deserialize;

// Import the generated LogTargetLevels struct
use crate::log::targets::LogTargetLevels;

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

    /// Enable slow statement logging
    #[serde(default)]
    pub slow_statements: bool,

    /// Threshold in milliseconds for slow statement logging (default: 2000ms)
    #[serde(default = "LogConfig::default_slow_statement_min_duration_ms")]
    pub slow_statement_min_duration_ms: u64,

    /// Threshold in milliseconds for slow database response logging (default: 100ms)
    #[serde(default = "LogConfig::default_slow_db_response_min_duration_ms")]
    pub slow_db_response_min_duration_ms: u64,

    // All log target levels - automatically generated and flattened from LogTargetLevels
    // To add a new target: just add it to the define_log_targets! macro in log/targets.rs
    #[serde(flatten)]
    pub targets: LogTargetLevels,
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
    /// The log config to use at startup.
    ///
    /// `explicit_level` is true when a level was set with `--log-level` or
    /// `CS_LOG__LEVEL`, and `interactive` when stdout is a terminal.
    pub fn for_startup(
        &self,
        explicit_level: bool,
        config_file_present: bool,
        debug: bool,
        interactive: bool,
    ) -> Self {
        if explicit_level {
            self.clone()
        } else if debug {
            self.with_default_level(LogLevel::Debug)
        } else if config_file_present || !interactive {
            self.clone()
        } else {
            self.with_default_level(LogLevel::Error)
        }
    }

    /// This config with `level` as the default level, used when no log level
    /// was set explicitly.
    /// Every other setting, and any per-target level that was set, is kept.
    pub fn with_default_level(&self, level: LogLevel) -> Self {
        LogConfig {
            level,
            targets: self.targets.with_default_level(level),
            ..self.clone()
        }
    }

    pub fn with_level(level: LogLevel) -> Self {
        LogConfig {
            format: LogConfig::default_log_format(),
            output: LogConfig::default_log_output(),
            ansi_enabled: LogConfig::default_ansi_enabled(),
            level,
            slow_statements: false,
            slow_statement_min_duration_ms: Self::default_slow_statement_min_duration_ms(),
            slow_db_response_min_duration_ms: Self::default_slow_db_response_min_duration_ms(),
            // All target levels automatically set using generated LogTargetLevels
            targets: LogTargetLevels::with_level(level),
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

    /// Default threshold for slow statement logging (2 seconds).
    ///
    /// This value represents a reasonable baseline for identifying slow queries in most
    /// PostgreSQL workloads. Queries exceeding this duration are likely candidates for
    /// optimization. Operators can adjust via CS_LOG__SLOW_STATEMENT_MIN_DURATION_MS.
    pub const fn default_slow_statement_min_duration_ms() -> u64 {
        2000
    }

    /// Default threshold for slow database response logging (100 milliseconds).
    ///
    /// This value represents the threshold for logging individual slow messages from
    /// the PostgreSQL server. Unlike slow statement logging which tracks total query
    /// time, this tracks per-message read latency from the server connection.
    /// Operators can adjust via CS_LOG__SLOW_DB_RESPONSE_MIN_DURATION_MS.
    pub const fn default_slow_db_response_min_duration_ms() -> u64 {
        100
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
                    TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert_eq!(config.log.level, LogLevel::Error);
            });

            temp_env::with_vars([("CS_LOG__LEVEL", Some("WARN"))], || {
                let config =
                    TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert_eq!(config.log.level, LogLevel::Warn);
            });

            temp_env::with_vars([("CS_LOG__OUTPUT", Some("stderr"))], || {
                let config =
                    TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert_eq!(config.log.output, LogOutput::Stderr);
            });

            temp_env::with_vars([("CS_LOG__FORMAT", Some("Pretty"))], || {
                let config =
                    TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert_eq!(config.log.format, LogFormat::Pretty);
            });

            temp_env::with_vars([("CS_LOG__FORMAT", Some("dEbUG"))], || {
                let config = TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml");

                assert!(config.is_err());
                assert!(matches!(config.unwrap_err(), Error::Config(_)));
            });
        });
    }

    #[test]
    /// The quiet default replaces only the level: settings and explicit
    /// per-target levels from env or config are kept.
    fn with_default_level_keeps_loaded_settings() {
        use crate::config::LogConfig;
        let mut loaded = LogConfig {
            output: LogOutput::Stderr,
            format: LogFormat::Structured,
            slow_statements: true,
            ..Default::default()
        };
        loaded.targets.mapper_level = LogLevel::Debug;

        let config = loaded.with_default_level(LogLevel::Error);

        assert_eq!(config.level, LogLevel::Error);
        assert_eq!(config.output, LogOutput::Stderr);
        assert_eq!(config.format, LogFormat::Structured);
        assert!(config.slow_statements);
        assert_eq!(config.targets.mapper_level, LogLevel::Debug);
        assert_eq!(config.targets.protocol_level, LogLevel::Error);
    }

    #[test]
    /// Containers run without a terminal, an explicit level or a config file,
    /// so the quiet default must not apply there.
    fn startup_keeps_loaded_level_when_not_interactive() {
        use crate::config::LogConfig;
        let config = LogConfig::default().for_startup(false, false, false, false);

        assert_eq!(config.level, LogConfig::default_log_level());
        assert_eq!(
            config.targets.protocol_level,
            LogConfig::default_log_level()
        );
    }

    #[test]
    fn startup_is_quiet_when_interactive() {
        use crate::config::LogConfig;
        let config = LogConfig::default().for_startup(false, false, false, true);

        assert_eq!(config.level, LogLevel::Error);
    }

    #[test]
    /// `--debug` is an explicit flag for this run, so it outranks a config
    /// file that sets no level.
    fn startup_debug_outranks_config_file() {
        use crate::config::LogConfig;
        let config = LogConfig::default().for_startup(false, true, true, true);

        assert_eq!(config.level, LogLevel::Debug);
    }

    #[test]
    fn startup_explicit_level_outranks_debug() {
        use crate::config::LogConfig;
        let loaded = LogConfig::with_level(LogLevel::Warn);

        let config = loaded.for_startup(true, false, true, true);

        assert_eq!(config.level, LogLevel::Warn);
    }
}
