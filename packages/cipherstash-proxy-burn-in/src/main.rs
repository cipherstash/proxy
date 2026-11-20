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
    /// Measure idle RSS growth of a released Proxy image, with zero client connections.
    RssIdle(RssIdleArgs),
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

#[derive(Debug, Args)]
struct RssIdleArgs {
    /// Released Proxy image to measure, unmodified.
    #[arg(long, default_value = "cipherstash/proxy:2.2.5")]
    image: String,
    /// PostgreSQL image for the TLS-only database Proxy connects to.
    #[arg(long, default_value = "postgres:17")]
    postgres_image: String,
    /// Wall-clock duration of each run, including warm-up.
    #[arg(long)]
    duration_seconds: u64,
    /// Samples before this point are recorded but excluded from the fitted slope.
    #[arg(long, default_value_t = 120)]
    warmup_seconds: u64,
    /// Seconds between `smaps_rollup` samples of the Proxy process.
    #[arg(long, default_value_t = 10)]
    sample_interval_seconds: u64,
    /// Reload interval applied to both the schema and encrypt-config reloaders.
    /// Each value is a separate run with its own containers; runs execute concurrently.
    #[arg(long = "reload-interval-seconds", default_values_t = [60, 1])]
    reload_intervals: Vec<u64>,
    /// Memory limit of each Proxy container, in `docker run --memory` syntax.
    #[arg(long, default_value = "512m")]
    memory_limit: String,
    /// Extra `NAME=VALUE` environment for Proxy, for example `MALLOC_ARENA_MAX=2`.
    #[arg(long = "proxy-env", value_parser = parse_env)]
    proxy_env: Vec<(String, String)>,
    /// JSON report containing the fitted slopes and the raw sample series.
    #[arg(long, default_value = "target/burn-in/rss-idle-report.json")]
    output: PathBuf,
    /// Leave the containers running after the measurement, for inspection.
    #[arg(long)]
    keep_containers: bool,
}

fn parse_env(value: &str) -> Result<(String, String), String> {
    match value.split_once('=') {
        Some((name, value)) if !name.is_empty() => Ok((name.to_string(), value.to_string())),
        _ => Err("expected NAME=VALUE".to_string()),
    }
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
        Command::RssIdle(args) => {
            cipherstash_proxy_burn_in::image_rig::run(
                cipherstash_proxy_burn_in::image_rig::Config {
                    image: args.image,
                    postgres_image: args.postgres_image,
                    duration: Duration::from_secs(args.duration_seconds),
                    warmup: Duration::from_secs(args.warmup_seconds),
                    sample_interval: Duration::from_secs(args.sample_interval_seconds),
                    reload_intervals: args.reload_intervals,
                    memory_limit: args.memory_limit,
                    proxy_env: args.proxy_env,
                    output: args.output,
                    keep_containers: args.keep_containers,
                },
            )
            .await
        }
    }
}
