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
