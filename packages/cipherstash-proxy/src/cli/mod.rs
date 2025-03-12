mod migrate;

use crate::{
    config::{LogConfig, LogFormat, LogLevel},
    error::Error,
    log::MIGRATE,
    TandemConfig,
};
use clap::{Parser, Subcommand};
use tracing::debug;

pub use migrate::Migrate;

const DEFAULT_CONFIG_FILE: &str = "cipherstash-proxy.toml";

#[derive(Clone, Debug, Parser)]
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
    pub command: Option<Commands>,
}

#[derive(Clone, Debug, Subcommand)]

pub enum Commands {
    Encrypt(Migrate),
}

pub async fn run(args: Args, config: TandemConfig) -> Result<(), Error> {
    match args.command {
        Some(Commands::Encrypt(migrate)) => {
            debug!(target: MIGRATE, ?migrate);
            migrate.run(config).await?;
            std::process::exit(exitcode::OK);
        }
        None => {}
    }
    Ok(())
}
