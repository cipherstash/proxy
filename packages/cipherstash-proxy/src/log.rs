use std::sync::Once;
use tracing_subscriber::filter::{Directive, EnvFilter};
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

fn subscriber_builder() -> SubscriberBuilder<DefaultFields, Format, EnvFilter> {
    // TODO: assign level from args
    let log_level: Directive = tracing::Level::DEBUG.into();

    let mut filter = EnvFilter::from_default_env().add_directive(log_level.to_owned());

    let directive = format!("eql_mapper={log_level}").parse().expect("ok");
    filter = filter.add_directive(directive);

    let directive = format!("{}={log_level}", AUTHENTICATION)
        .parse()
        .expect("ok");
    filter = filter.add_directive(directive);

    let log_level: Directive = tracing::Level::DEBUG.into();
    let directive = format!("{}={log_level}", CONTEXT).parse().expect("ok");
    filter = filter.add_directive(directive);

    let log_level: Directive = tracing::Level::DEBUG.into();
    let directive = format!("{}={log_level}", DEVELOPMENT).parse().expect("ok");
    filter = filter.add_directive(directive);

    let log_level: Directive = tracing::Level::DEBUG.into();
    let directive = format!("{}={log_level}", KEYSET).parse().expect("ok");
    filter = filter.add_directive(directive);

    let log_level: Directive = tracing::Level::DEBUG.into();
    let directive = format!("{}={log_level}", MAPPER).parse().expect("ok");
    filter = filter.add_directive(directive);

    let log_level: Directive = tracing::Level::DEBUG.into();
    let directive = format!("{}={log_level}", PROTOCOL).parse().expect("ok");
    filter = filter.add_directive(directive);

    let log_level: Directive = tracing::Level::DEBUG.into();
    let directive = format!("{}={log_level}", SCHEMA).parse().expect("ok");
    filter = filter.add_directive(directive);

    FmtSubscriber::builder()
        .with_env_filter(filter)
        .with_file(true)
        .with_line_number(true)
        .with_target(true)
}

pub fn init() {
    INIT.call_once(|| {
        let subscriber = subscriber_builder().finish();

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
    use tracing_subscriber::fmt::MakeWriter;
    use tracing::dispatcher::set_default;
    use tracing::info;

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
        let subscriber = subscriber_builder()
            .with_writer(make_writer.clone())
            .finish();
        let _default = set_default(&subscriber.into());

        info!("info message");

        let log_contents = make_writer.get_string();
        assert!(log_contents.contains("info message"));
    }
}
