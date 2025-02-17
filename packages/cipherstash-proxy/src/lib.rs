#![allow(dead_code)]

pub mod config;
pub mod connect;
pub mod encrypt;
pub mod eql;
pub mod error;
pub mod log;
pub mod postgresql;
pub mod prometheus;
pub mod tls;

pub use crate::config::{DatabaseConfig, ServerConfig, TandemConfig, TlsConfig};
pub use crate::encrypt::Encrypt;
pub use crate::eql::{Encrypted, ForQuery, Identifier, Plaintext};
pub use crate::log::init;

use std::mem;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const SIZE_U8: usize = mem::size_of::<u8>();
pub const SIZE_I16: usize = mem::size_of::<i16>();
pub const SIZE_I32: usize = mem::size_of::<i32>();
