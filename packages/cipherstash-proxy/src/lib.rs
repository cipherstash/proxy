#![allow(dead_code)]

pub mod cli;
pub mod config;
pub mod connect;
pub mod error;
pub mod log;
pub mod postgresql;
pub mod prometheus;
pub mod proxy;
pub mod tls;

pub use crate::cli::Args;
pub use crate::cli::Migrate;
pub use crate::config::{DatabaseConfig, ServerConfig, TandemConfig, TlsConfig};
pub use crate::log::init;
pub use crate::proxy::Proxy;
pub use cipherstash_client::encryption::Plaintext;
pub use cipherstash_client::eql::{EqlCiphertext, Identifier};

use std::mem;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const EQL_SCHEMA_VERSION: u16 = 2;

pub const SIZE_U8: usize = mem::size_of::<u8>();
pub const SIZE_I16: usize = mem::size_of::<i16>();
pub const SIZE_I32: usize = mem::size_of::<i32>();

#[cfg(test)]
pub mod test_helpers;
