use crate::config::{LogConfig, LogLevel, LogOutput};
use crate::log::{AUTHENTICATION, CONTEXT, DEVELOPMENT, ENCRYPT, KEYSET, MAPPER, PROTOCOL, SCHEMA};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::{DefaultFields, Format};
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::fmt::SubscriberBuilder;
use tracing_subscriber::FmtSubscriber;

use super::DECRYPT;

fn log_targets() -> Vec<&'static str> {
    vec![
        DEVELOPMENT,
        AUTHENTICATION,
        CONTEXT,
        DECRYPT,
        ENCRYPT,
        KEYSET,
        PROTOCOL,
        MAPPER,
        SCHEMA,
    ]
}

fn log_level_for(config: &LogConfig, target: &str) -> LogLevel {
    match target {
        DEVELOPMENT => config.development_level,
        AUTHENTICATION => config.authentication_level,
        CONTEXT => config.context_level,
        DECRYPT => config.decrypt_level,
        ENCRYPT => config.encrypt_level,
        KEYSET => config.keyset_level,
        PROTOCOL => config.protocol_level,
        MAPPER => config.mapper_level,
        SCHEMA => config.schema_level,
        _ => config.level,
    }
}

pub fn builder(
    config: &LogConfig,
) -> SubscriberBuilder<DefaultFields, Format, EnvFilter, BoxMakeWriter> {
    let log_level = config.level.to_owned();

    let mut env_filter: EnvFilter = EnvFilter::builder().parse_lossy(log_level.to_string());

    let mut debug = is_debug(&log_level);

    for &target in log_targets().iter() {
        let level = log_level_for(config, target);

        // If any level is debug, enable debug mode
        if is_debug(&level) {
            debug = true;
        }

        env_filter = env_filter.add_directive(format!("{target}={level}").parse().unwrap());
    }

    let writer = match config.output {
        LogOutput::Stderr => BoxMakeWriter::new(std::io::stderr),
        LogOutput::Stdout => BoxMakeWriter::new(std::io::stdout),
    };

    let mut builder = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_ansi(config.ansi_enabled)
        .with_writer(writer);

    if debug {
        builder = builder
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true);
    };

    builder
}

fn is_debug(level: &LogLevel) -> bool {
    matches!(level, LogLevel::Debug | LogLevel::Trace)
}
