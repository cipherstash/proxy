use crate::config::LogLevel;
use serde::Deserialize;

// Define all log targets in one place
macro_rules! define_log_targets {
    ($(($const_name:ident, $field_name:ident)),* $(,)?) => {
        // Generate target constants with automatic string generation
        $(
            pub const $const_name: &str = stringify!($const_name);
        )*

        // Generate LogTargetLevels struct with all target level fields
        #[derive(Clone, Debug, Deserialize)]
        pub struct LogTargetLevels {
            $(
                #[serde(default = "LogTargetLevels::default_target_level")]
                pub $field_name: LogLevel,
            )*
        }

        impl Default for LogTargetLevels {
            fn default() -> Self {
                Self::with_level(LogLevel::Info)
            }
        }

        impl LogTargetLevels {
            pub fn with_level(level: LogLevel) -> Self {
                LogTargetLevels {
                    $(
                        $field_name: level,
                    )*
                }
            }

            pub const fn default_target_level() -> LogLevel {
                LogLevel::Info
            }

            pub fn get_level_for_target(&self, target: &str) -> LogLevel {
                match target {
                    $(
                        $const_name => self.$field_name,
                    )*
                    _ => LogLevel::Info, // fallback level
                }
            }
        }

        // Generate function to get all target names
        pub fn log_targets() -> Vec<&'static str> {
            vec![
                $(
                    $const_name,
                )*
            ]
        }

        // Generate function to map target to log level using the flattened config
        pub fn log_level_for(config: &crate::config::LogConfig, target: &str) -> LogLevel {
            config.targets.get_level_for_target(target)
        }
    };
}

define_log_targets!(
    (DEVELOPMENT, development_level),
    (AUTHENTICATION, authentication_level),
    (CONFIG, config_level),
    (CONTEXT, context_level),
    (ENCODING, encoding_level),
    (ENCRYPT, encrypt_level),
    (DECRYPT, decrypt_level),
    (ENCRYPT_CONFIG, encrypt_config_level),
    (ZEROKMS, zerokms_level),
    (MIGRATE, migrate_level),
    (PROTOCOL, protocol_level),
    (MAPPER, mapper_level),
    (SCHEMA, schema_level),
    (SLOW_STATEMENTS, slow_statements_level),
);
