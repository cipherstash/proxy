pub mod subscriber;

use crate::config::{LogConfig, LogFormat};
use std::sync::Once;
use tracing_subscriber::{
    fmt::{
        format::{DefaultFields, Format},
        writer::BoxMakeWriter,
        SubscriberBuilder,
    },
    EnvFilter,
};

// Log targets used in logs like `debug!(target: DEVELOPMENT, "Flush message buffer");`
// If you add one, make sure `log_targets()` and `log_level_for()` functions are updated.
pub const DEVELOPMENT: &str = "development"; // one for various hidden "development mode" messages
pub const AUTHENTICATION: &str = "authentication";
pub const CONFIG: &str = "config";
pub const CONTEXT: &str = "context";
pub const ENCRYPT: &str = "encrypt";
pub const DECRYPT: &str = "decrypt";
pub const ENCODING: &str = "encoding";
pub const ENCRYPT_CONFIG: &str = "encrypt_config";
pub const KEYSET: &str = "keyset";
pub const MIGRATE: &str = "migrate";
pub const PARSER: &str = "parser";
pub const PROTOCOL: &str = "protocol";
pub const MAPPER: &str = "mapper";
pub const SCHEMA: &str = "schema";

static INIT: Once = Once::new();

type Subscriber = Box<dyn tracing::Subscriber + Send + Sync>;

pub fn init(config: LogConfig) {
    INIT.call_once(|| {
        let subscriber = subscriber::builder(&config);
        let subscriber = set_format(&config, subscriber);

        tracing::subscriber::set_global_default(subscriber)
            .expect("Could not set the tracing subscriber");
    });
}

pub fn set_format(
    config: &LogConfig,
    builder: SubscriberBuilder<DefaultFields, Format, EnvFilter, BoxMakeWriter>,
) -> Subscriber {
    match &config.format {
        LogFormat::Pretty => Box::new(builder.pretty().finish()),
        LogFormat::Structured => Box::new(builder.json().finish()),
        LogFormat::Text => Box::new(builder.finish()),
    }
}

#[cfg(test)]
mod tests {
    use crate::config::LogLevel;

    use super::*;
    use tracing::dispatcher::set_default;
    use tracing::{debug, error, info, trace, warn};
    use crate::test_helpers::MockMakeWriter;

    #[test]
    fn test_simple_log() {
        let make_writer = MockMakeWriter::default();

        let config = LogConfig::default();

        let subscriber =
            subscriber::builder(&config).with_writer(BoxMakeWriter::new(make_writer.clone()));

        let subscriber = set_format(&config, subscriber);

        let _default = set_default(&subscriber.into());

        error!("error message");

        let log_contents = make_writer.get_string();
        assert!(log_contents.contains("error message"));
    }

    #[test]
    fn test_log_levels() {
        let make_writer = MockMakeWriter::default();

        let config = LogConfig::with_level(LogLevel::Warn);

        let subscriber =
            subscriber::builder(&config).with_writer(BoxMakeWriter::new(make_writer.clone()));

        let subscriber = set_format(&config, subscriber);

        let _default = set_default(&subscriber.into());

        trace!("trace message");
        debug!("debug message");
        info!("info message");
        warn!("warn message");
        error!("error message");

        let log_contents = make_writer.get_string();
        assert!(!log_contents.contains("trace message"));
        assert!(!log_contents.contains("debug message"));
        assert!(!log_contents.contains("info message"));
        assert!(log_contents.contains("warn message"));
        assert!(log_contents.contains("error message"));
    }

    // test info level with debug target and error target
    #[test]
    fn test_log_levels_with_targets() {
        let make_writer = MockMakeWriter::default();

        let config = LogConfig {
            format: LogConfig::default_log_format(),
            output: LogConfig::default_log_output(),
            ansi_enabled: LogConfig::default_ansi_enabled(),
            level: LogLevel::Info,
            development_level: LogLevel::Info,
            authentication_level: LogLevel::Debug,
            context_level: LogLevel::Error,
            encoding_level: LogLevel::Error,
            encrypt_level: LogLevel::Error,
            encrypt_config_level: LogLevel::Error,
            decrypt_level: LogLevel::Error,
            keyset_level: LogLevel::Trace,
            migrate_level: LogLevel::Trace,
            protocol_level: LogLevel::Info,
            mapper_level: LogLevel::Info,
            schema_level: LogLevel::Info,
            config_level: LogLevel::Info,
        };

        let subscriber =
            subscriber::builder(&config).with_writer(BoxMakeWriter::new(make_writer.clone()));

        let subscriber = set_format(&config, subscriber);

        let _default = set_default(&subscriber.into());

        // with development level 'info', info should be logged but not debug
        debug!(target: "development", "debug/development");
        info!(target: "development", "info/development");
        let log_contents = make_writer.get_string();
        assert!(!log_contents.contains("debug/development"));
        assert!(log_contents.contains("info/development"));

        // with authentication level 'debug', debug should be logged but not trace
        trace!(target: "authentication", "trace/authentication");
        debug!(target: "authentication", "debug/authentication");
        let log_contents = make_writer.get_string();
        assert!(!log_contents.contains("trace/authentication"));
        assert!(log_contents.contains("debug/authentication"));

        // with context level 'error', error should be logged but not warn
        warn!(target: "context", "warn/context");
        error!(target: "context", "error/context");
        let log_contents = make_writer.get_string();
        assert!(!log_contents.contains("warn/context"));
        assert!(log_contents.contains("error/context"));

        // with keyset level 'trace', trace should be logged
        trace!(target: "keyset", "trace/keyset");
        let log_contents = make_writer.get_string();
        assert!(log_contents.contains("trace/keyset"));

        // with protocol level 'info', info should be logged but not debug
        debug!(target: "protocol", "debug/protocol");
        info!(target: "protocol", "info/protocol");
        let log_contents = make_writer.get_string();
        assert!(!log_contents.contains("debug/protocol"));
        assert!(log_contents.contains("info/protocol"));

        // with mapper level 'info', info should be logged but not debug
        debug!(target: "mapper", "debug/mapper");
        info!(target: "mapper", "info/mapper");
        let log_contents = make_writer.get_string();
        assert!(!log_contents.contains("debug/mapper"));
        assert!(log_contents.contains("info/mapper"));

        // with schema level 'info', info should be logged but not debug
        debug!(target: "schema", "debug/schema");
        info!(target: "schema", "info/schema");
        let log_contents = make_writer.get_string();
        assert!(!log_contents.contains("debug/schema"));
        assert!(log_contents.contains("info/schema"));
    }

    #[test]
    fn test_log_format_structured() {
        let make_writer = MockMakeWriter::default();

        let mut config = LogConfig::with_level(LogLevel::Info);
        config.format = LogFormat::Structured;

        let subscriber =
            subscriber::builder(&config).with_writer(BoxMakeWriter::new(make_writer.clone()));

        let subscriber = set_format(&config, subscriber);

        let _default = set_default(&subscriber.into());

        info!(msg = "message", value = 42);

        let log_contents = make_writer.get_string();

        assert!(log_contents.contains(r#"fields":{"msg":"message","value":42}"#));
    }
}
