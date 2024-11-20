pub mod config;
pub mod encrypt;
pub mod eql;
pub mod error;
pub mod postgresql;

pub use crate::config::TandemConfig;

use std::{mem, sync::Once};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

static INIT: Once = Once::new();

const SIZE_U8: usize = mem::size_of::<u8>();
const SIZE_I16: usize = mem::size_of::<i16>();
const SIZE_I32: usize = mem::size_of::<i32>();

pub fn trace() {
    INIT.call_once(|| {
        use tracing_subscriber::FmtSubscriber;

        let subscriber = FmtSubscriber::builder()
            .with_max_level(tracing::Level::DEBUG) // Set the maximum level of tracing events that should be logged.
            .with_file(true)
            .with_line_number(true)
            .with_target(true)
            .finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    });
}
