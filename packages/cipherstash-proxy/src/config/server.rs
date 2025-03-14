use super::{DEFAULT_PORT, DEFAULT_SHUTDOWN_TIMEOUT, DEFAULT_WORKER_THREADS};
use crate::error::{ConfigError, Error};
use rustls_pki_types::ServerName;
use serde::Deserialize;
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "ServerConfig::default_host")]
    pub host: String,

    #[serde(default = "ServerConfig::default_port")]
    pub port: u16,

    #[serde(default)]
    pub require_tls: bool,

    #[serde(default = "ServerConfig::default_shutdown_timeout")]
    pub shutdown_timeout: u64,

    #[serde(default = "ServerConfig::default_worker_threads")]
    pub worker_threads: usize,

    pub thread_stack_size: Option<usize>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            host: ServerConfig::default_host(),
            port: ServerConfig::default_port(),
            require_tls: false,
            shutdown_timeout: ServerConfig::default_shutdown_timeout(),
            worker_threads: ServerConfig::default_worker_threads(),
            thread_stack_size: None,
        }
    }
}

impl ServerConfig {
    pub fn default_host() -> String {
        "0.0.0.0".to_string()
    }

    pub fn default_port() -> u16 {
        DEFAULT_PORT
    }

    pub fn default_shutdown_timeout() -> u64 {
        DEFAULT_SHUTDOWN_TIMEOUT
    }

    ///
    /// Default number of worker threads
    /// This is half the number of available cores or DEFAULT_WORKER_THREADS, whichever is greater
    pub fn default_worker_threads() -> usize {
        match thread::available_parallelism() {
            Ok(p) => {
                let count = p.get();
                let threads = count / 2;
                threads.max(DEFAULT_WORKER_THREADS)
            }
            Err(_) => DEFAULT_WORKER_THREADS,
        }
    }

    pub fn server_name(&self) -> Result<ServerName, Error> {
        let name = ServerName::try_from(self.host.as_str()).map_err(|_| {
            ConfigError::InvalidServerName {
                name: self.host.to_owned(),
            }
        })?;
        Ok(name)
    }

    pub fn to_socket_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn shutdown_timeout(&self) -> Duration {
        Duration::from_millis(self.shutdown_timeout)
    }
}
