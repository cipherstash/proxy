use crate::config::{LogConfig, LogFormat, LogLevel};
use clap::{Parser, Subcommand};

const DEFAULT_CONFIG_FILE: &str = "cipherstash-proxy.toml";

#[derive(Parser, Debug)]
#[command(version, about, verbatim_doc_comment)]
///
/// CipherStash Proxy
///
/// CipherStash Proxy keeps your sensitive data in PostgreSQL encrypted and searchable, with no changes to SQL.
///
pub struct Args {
    /// Optional path to a CipherStash Proxy configuration file.
    ///
    /// Default is "cipherstash-proxy.toml".
    /// Configuration is loaded from this file, if present.
    /// Environment variables are used instead of the file or to override any values defined in the file.
    #[arg(short, long, default_value = DEFAULT_CONFIG_FILE, verbatim_doc_comment)]
    pub config_file: String,

    ///
    /// Optional log level.
    ///
    #[arg(short, long, value_enum, default_value_t = LogConfig::default_log_level(), env = "CS_LOG__LEVEL")]
    pub log_level: LogLevel,

    ///
    /// Optional log format. Default level is "pretty" if running in a terminal session, otherwise "structured".
    ///
    #[arg(short='f', long, value_enum, default_value_t = LogConfig::default_log_format(), env = "CS_LOG__FORMAT")]
    pub log_format: LogFormat,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
///
/// A `help` subcommand is automatically generated but ONLY if there are other subcommands.
/// Noop is not visible in help, but enables proxy to be called as `cipherstash-proxy help` to show help
enum Commands {
    #[command(hide = true)]
    Noop,
}
