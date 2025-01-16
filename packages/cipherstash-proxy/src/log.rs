use crate::config::LogConfig;
use std::sync::Once;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::{DefaultFields, Format};
use tracing_subscriber::fmt::SubscriberBuilder;
use tracing_subscriber::FmtSubscriber;

static INIT: Once = Once::new();
// Messages related to the various hidden "development mode" messages
pub const DEVELOPMENT: &str = "development";

pub const AUTHENTICATION: &str = "authentication";
pub const CONTEXT: &str = "context";
pub const KEYSET: &str = "keyset";
pub const PROTOCOL: &str = "protocol";
pub const MAPPER: &str = "mapper";
pub const SCHEMA: &str = "schema";

fn subscriber_builder(
    default_log_level: &str,
    config: Option<LogConfig>,
) -> SubscriberBuilder<DefaultFields, Format, EnvFilter> {
    let mut env_filter: EnvFilter = EnvFilter::builder().parse_lossy(default_log_level);
    for &target in [
        DEVELOPMENT,
        AUTHENTICATION,
        CONTEXT,
        KEYSET,
        PROTOCOL,
        MAPPER,
        SCHEMA,
    ]
    .iter()
    {
        if let Some(level) = config.as_ref().map(|c| match target {
            DEVELOPMENT => &c.development_level,
            AUTHENTICATION => &c.authentication_level,
            CONTEXT => &c.context_level,
            KEYSET => &c.keyset_level,
            PROTOCOL => &c.protocol_level,
            MAPPER => &c.mapper_level,
            SCHEMA => &c.schema_level,
            _ => default_log_level,
        }) {
            env_filter = env_filter.add_directive(format!("{target}={level}").parse().unwrap());
        }
    }

    FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_file(true)
        .with_line_number(true)
        .with_target(true)
}

pub fn init(config: Option<crate::config::LogConfig>) {
    INIT.call_once(|| {
        let log_level = std::env::var("RUST_LOG").unwrap_or("info".into());
        let subscriber = subscriber_builder(log_level.as_str(), config).finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::{
        io,
        sync::{MutexGuard, TryLockError},
    };
    use tracing::dispatcher::set_default;
    use tracing::{debug, error, info, trace, warn};
    use tracing_subscriber::fmt::MakeWriter;

    // Mock Writer for flexibly testing the logging behaviour, copy-pasted from
    // tracing_subscriber's internal test code (with JSON functionality deleted).
    // https://github.com/tokio-rs/tracing/blob/b02a700ba6850ad813f77e65144114f866074a8f/tracing-subscriber/src/fmt/mod.rs#L1247-L1314
    pub(crate) struct MockWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl MockWriter {
        pub(crate) fn new(buf: Arc<Mutex<Vec<u8>>>) -> Self {
            Self { buf }
        }

        pub(crate) fn map_error<Guard>(err: TryLockError<Guard>) -> io::Error {
            match err {
                TryLockError::WouldBlock => io::Error::from(io::ErrorKind::WouldBlock),
                TryLockError::Poisoned(_) => io::Error::from(io::ErrorKind::Other),
            }
        }

        pub(crate) fn buf(&self) -> io::Result<MutexGuard<'_, Vec<u8>>> {
            self.buf.try_lock().map_err(Self::map_error)
        }
    }

    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buf()?.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.buf()?.flush()
        }
    }

    #[derive(Clone, Default)]
    pub(crate) struct MockMakeWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl MockMakeWriter {
        pub(crate) fn new(buf: Arc<Mutex<Vec<u8>>>) -> Self {
            Self { buf }
        }

        pub(crate) fn get_string(&self) -> String {
            let mut buf = self.buf.lock().expect("lock shouldn't be poisoned");
            let string = std::str::from_utf8(&buf[..])
                .expect("formatter should not have produced invalid utf-8")
                .to_owned();
            buf.clear();
            string
        }
    }

    impl<'a> MakeWriter<'a> for MockMakeWriter {
        type Writer = MockWriter;

        fn make_writer(&'a self) -> Self::Writer {
            MockWriter::new(self.buf.clone())
        }
    }

    #[test]
    fn test_simple_log() {
        let make_writer = MockMakeWriter::default();
        let subscriber = subscriber_builder("info", None)
            .with_writer(make_writer.clone())
            .finish();
        let _default = set_default(&subscriber.into());

        error!("error message");

        let log_contents = make_writer.get_string();
        assert!(log_contents.contains("error message"));
    }

    #[test]
    fn test_log_levels() {
        let make_writer = MockMakeWriter::default();
        let subscriber = subscriber_builder("warn", None)
            .with_writer(make_writer.clone())
            .finish();
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
        let config = Some(LogConfig {
            development_level: "info".into(),
            authentication_level: "debug".into(),
            context_level: "error".into(),
            keyset_level: "trace".into(),
            protocol_level: "info".into(),
            mapper_level: "info".into(),
            schema_level: "info".into(),
        });
        let make_writer = MockMakeWriter::default();
        let subscriber = subscriber_builder("warn", config)
            .with_writer(make_writer.clone())
            .finish();
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
}
