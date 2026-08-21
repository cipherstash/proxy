//! CLI entry point for deterministic conformance and timed burn-in runs.
//!
//! Connection and EQL paths are explicit options with environment-variable
//! equivalents so the same binary works in local mise environments and CI.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use cipherstash_proxy_burn_in::database::DatabaseTarget;
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "cipherstash-proxy-burn-in",
    about = "Conformance and release-mode soak testing for CipherStash Proxy"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run deterministic correctness and PostgreSQL-protocol scenarios.
    Conformance(DatabaseArgs),
    /// Build and start the proxy in release mode, then run a timed stress workload.
    Soak(SoakArgs),
}

#[derive(Debug, Args)]
struct DatabaseArgs {
    /// Connection URL through CipherStash Proxy.
    #[arg(
        long,
        env = "BURN_IN_PROXY_DATABASE_URL",
        hide_env_values = true,
        default_value = "postgresql://cipherstash:p%40ssword@localhost:6432/cipherstash"
    )]
    proxy_database_url: DatabaseTarget,
    /// Direct PostgreSQL URL used only to install and seed the fixture schema.
    #[arg(
        long,
        env = "BURN_IN_DIRECT_DATABASE_URL",
        hide_env_values = true,
        default_value = "postgresql://cipherstash:p%40ssword@localhost:5532/cipherstash"
    )]
    direct_database_url: DatabaseTarget,
    /// EQL installation SQL used when the target database has no EQL domains.
    #[arg(
        long,
        env = "BURN_IN_EQL_PATH",
        default_value = "cipherstash-encrypt.sql"
    )]
    eql_path: PathBuf,
}

#[derive(Debug, Args)]
struct SoakArgs {
    #[command(flatten)]
    database: DatabaseArgs,
    /// Wall-clock duration of the stress workload.
    #[arg(long)]
    duration_seconds: u64,
    /// Number of concurrent long-lived database connections.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
    /// JSON report containing operation counts and one-second RSS samples.
    #[arg(long, default_value = "target/burn-in/soak-report.json")]
    output: PathBuf,
    /// Optional hard gate for end-to-end proxy RSS growth, in MiB.
    #[arg(long)]
    max_rss_growth_mib: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Conformance(args) => {
            cipherstash_proxy_burn_in::conformance::run(
                &args.proxy_database_url,
                &args.direct_database_url,
                &args.eql_path,
            )
            .await
        }
        Command::Soak(args) => {
            let max_rss_growth_bytes = args
                .max_rss_growth_mib
                .map(|mib| {
                    mib.checked_mul(1_048_576)
                        .ok_or_else(|| anyhow::anyhow!("--max-rss-growth-mib is too large"))
                })
                .transpose()?;
            cipherstash_proxy_burn_in::soak::run(cipherstash_proxy_burn_in::soak::Config {
                duration: Duration::from_secs(args.duration_seconds),
                concurrency: args.concurrency,
                proxy_database_url: args.database.proxy_database_url,
                direct_database_url: args.database.direct_database_url,
                eql_path: args.database.eql_path,
                output: args.output,
                max_rss_growth_bytes,
            })
            .await
        }
    }
}
