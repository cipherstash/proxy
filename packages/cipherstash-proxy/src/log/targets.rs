use crate::config::LogLevel;

// Define all log targets in one place
macro_rules! define_log_targets {
    ($(($const_name:ident, $field_name:ident, $target_str:literal)),* $(,)?) => {
        // Generate target constants
        $(
            pub const $const_name: &str = $target_str;
        )*

        // Generate function to get all target names
        pub fn log_targets() -> Vec<&'static str> {
            vec![
                $(
                    $const_name,
                )*
            ]
        }

        // Generate function to map target to log level
        pub fn log_level_for(config: &crate::config::LogConfig, target: &str) -> LogLevel {
            match target {
                $(
                    $const_name => config.$field_name,
                )*
                _ => config.level,
            }
        }

        // Compile-time validation that LogConfig has all required fields
        // This will fail to compile if any field is missing from LogConfig
        pub const fn validate_log_config_fields() {
            use crate::config::LogConfig;

            // Create a dummy LogConfig to ensure all fields exist
            let _config = LogConfig {
                ansi_enabled: true,
                format: crate::config::LogFormat::Pretty,
                output: crate::config::LogOutput::Stdout,
                level: LogLevel::Info,
                $(
                    $field_name: LogLevel::Info,
                )*
            };
        }

        // NOTE: Due to Rust macro system limitations, LogConfig fields in config/log.rs
        // must be manually synchronized with the targets defined here.
        //
        // When adding a new target (NEWTARGET, new_target_level, "new_target"):
        // 1. Add the target to the define_log_targets! macro invocation below
        // 2. Add this field to LogConfig struct in config/log.rs:
        //    #[serde(default = "LogConfig::default_log_level")]
        //    pub new_target_level: LogLevel,
        // 3. Add this assignment to with_level() method in config/log.rs:
        //    new_target_level: level,
        //
        // The validate_log_config_fields() function will fail to compile if fields are missing.
    };
}

define_log_targets!(
    (DEVELOPMENT, development_level, "development"),
    (AUTHENTICATION, authentication_level, "authentication"),
    (CONFIG, config_level, "config"),
    (CONTEXT, context_level, "context"),
    (ENCODING, encoding_level, "encoding"),
    (ENCRYPT, encrypt_level, "encrypt"),
    (DECRYPT, decrypt_level, "decrypt"),
    (ENCRYPT_CONFIG, encrypt_config_level, "encrypt_config"),
    (KEYSET, keyset_level, "keyset"),
    (MIGRATE, migrate_level, "migrate"),
    (PROTOCOL, protocol_level, "protocol"),
    (PROXY, proxy_level, "proxy"),
    (MAPPER, mapper_level, "mapper"),
    (SCHEMA, schema_level, "schema"),
);

// Trigger compile-time validation
const _: () = validate_log_config_fields();
