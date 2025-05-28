use std::io;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
/// This module contains test helpers
use temp_env;
use tracing_subscriber::fmt::MakeWriter;

/// Runs a function with all CS_ environment variables unset
pub(crate) fn with_no_cs_vars<F: FnOnce() -> R, R>(f: F) -> R {
    let cs_vars = std::env::vars()
        .map(|(k, _v)| k)
        .filter(|k| k.starts_with("CS_"))
        .collect::<Vec<_>>();

    temp_env::with_vars_unset(&cs_vars, f)
}

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
