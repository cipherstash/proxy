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
    /// Optional full PostgreSQL connection string, e.g.
    /// "postgres://user:pass@host:5432/dbname".
    /// Sets host, port, user, password and database name at once.
    /// Individual flags below override matching parts of the URL.
    #[arg(long, value_name = "URL", verbatim_doc_comment)]
    pub database_url: Option<String>,

    /// Optional database host to connect to.
    /// Uses env or config file if not specified.
    #[arg(short = 'H', long)]
    pub db_host: Option<String>,

    /// Optional database port to connect to.
    /// Uses env or config file if not specified.
    #[arg(short = 'P', long)]
    pub db_port: Option<u16>,

    /// Optional database name to connect to.
    /// Uses env or config file if not specified.
    #[arg(value_name = "DBNAME")]
    pub db_name: Option<String>,

    /// Optional database user to connect as.
    /// Uses env or config file if not specified.
    #[arg(short = 'u', long)]
    pub db_user: Option<String>,

    /// Optional database password.
    /// Uses env or config file if not specified.
    /// Prefer CS_DATABASE__PASSWORD or --database-url to avoid leaking the
    /// password into shell history / the process list.
    #[arg(short = 'W', long, verbatim_doc_comment)]
    pub db_password: Option<String>,

    /// Disable inbound (client-facing) TLS: the proxy listens for client
    /// connections in plaintext. Overrides any CS_TLS__* env / config.
    /// Use for local development only. Does not affect the connection from the
    /// proxy to the database.
    #[arg(long, verbatim_doc_comment)]
    pub no_tls: bool,

    /// Optional path to a CipherStash Proxy configuration file.
    ///
    /// Default is "cipherstash-proxy.toml".
    /// Configuration is loaded from this file, if present.
    /// Environment variables are used instead of the file or to override any values defined in the file.
    #[arg(short = 'p', long, default_value = DEFAULT_CONFIG_FILE, verbatim_doc_comment, global = true)]
    pub config_file_path: String,

    ///
    /// Optional log level.
    ///
    #[arg(short, long, value_enum, default_value_t = LogConfig::default_log_level(), env = "CS_LOG__LEVEL", global = true)]
    pub log_level: LogLevel,

    ///
    /// Optional log format. Default level is "pretty" if running in a terminal session, otherwise "structured".
    ///
    #[arg(short='f', long, value_enum, default_value_t = LogConfig::default_log_format(), env = "CS_LOG__FORMAT", global = true)]
    pub log_format: LogFormat,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Clone, Debug, Subcommand)]

pub enum Commands {
    Encrypt(Migrate),
}

///
/// Runs command specified in command line
/// Returns Ok(true) if the caller should exit
///
pub async fn run(args: Args, config: TandemConfig) -> Result<bool, Error> {
    match args.command {
        Some(Commands::Encrypt(migrate)) => {
            debug!(target: MIGRATE, ?migrate);
            migrate.run(config).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}
