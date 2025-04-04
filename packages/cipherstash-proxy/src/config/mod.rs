mod database;
mod log;
mod server;
mod tandem;
mod tls;

pub use database::DatabaseConfig;
pub use log::{LogConfig, LogFormat, LogLevel, LogOutput};
use serde::Deserialize;
pub use server::ServerConfig;
pub use tandem::TandemConfig;
pub use tls::TlsConfig;
use vitaminc_protected::Protected;

pub const CS_PREFIX: &str = "CS";

pub const DEFAULT_CONFIG_FILE_PATH: &str = "cipherstash-proxy.toml";

// 2 MiB
pub const DEFAULT_THREAD_STACK_SIZE: usize = 2 * 1024 * 1024;

// 4 MiB
pub const DEBUG_THREAD_STACK_SIZE: usize = 4 * 1024 * 1024;

pub const DEFAULT_PORT: u16 = 6432;
pub const DEFAULT_SHUTDOWN_TIMEOUT: u64 = 2000;
pub const DEFAULT_WORKER_THREADS: usize = 4;

fn protected_string_deserializer<'de, D>(deserializer: D) -> Result<Protected<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(Protected::new(s))
}
