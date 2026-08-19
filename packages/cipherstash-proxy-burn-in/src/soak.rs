//! Timed encrypted CRUD workload with release-Proxy RSS sampling.
//!
//! This module builds and owns the exact Proxy process being measured. Every
//! CRUD cycle writes and reads EQL-domain columns; fixture setup also verifies
//! ciphertext directly in PostgreSQL before timing begins. Memory growth
//! therefore includes the encryption/decryption path rather than passthrough
//! SQL alone.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::{
    process::{Child, Command},
    task::JoinSet,
};

use crate::{
    database,
    resource::{self, MemorySample},
};

#[derive(Debug)]
pub struct Config {
    pub duration: Duration,
    pub concurrency: usize,
    pub proxy_database_url: String,
    pub direct_database_url: String,
    pub eql_path: PathBuf,
    pub output: PathBuf,
    pub max_rss_growth_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
struct Report {
    duration_seconds: u64,
    operations: u64,
    errors: u64,
    proxy_pid: u32,
    initial_rss_bytes: u64,
    final_rss_bytes: u64,
    peak_rss_bytes: u64,
    rss_growth_bytes: u64,
    memory_samples: Vec<MemorySample>,
}

pub async fn run(config: Config) -> Result<()> {
    anyhow::ensure!(
        !config.duration.is_zero(),
        "--duration-seconds must be positive"
    );
    anyhow::ensure!(config.concurrency > 0, "--concurrency must be positive");
    database::ensure_eql_installed(&config.direct_database_url, &config.eql_path).await?;

    build_release_proxy().await?;
    let mut proxy = spawn_release_proxy()?;
    let proxy_pid = proxy.id().context("release proxy did not expose a PID")?;
    let result = run_with_proxy(&config, proxy_pid).await;
    let _ = proxy.kill().await;
    let _ = proxy.wait().await;
    result
}

async fn run_with_proxy(config: &Config, proxy_pid: u32) -> Result<()> {
    database::wait_until_ready(&config.proxy_database_url).await?;
    database::migrate(&config.proxy_database_url, &config.direct_database_url).await?;
    let started_at = Instant::now();
    let deadline = tokio::time::Instant::now() + config.duration;
    let operations = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let ids = Arc::new(AtomicU64::new(1_000_000));
    let mut workers = JoinSet::new();

    // Workers keep connections open so the soak stresses repeated statement
    // mapping and cipher use rather than connection establishment throughput.
    for _ in 0..config.concurrency {
        let url = config.proxy_database_url.clone();
        let operations = Arc::clone(&operations);
        let errors = Arc::clone(&errors);
        let ids = Arc::clone(&ids);
        workers.spawn(async move {
            let mut client = database::connect(&url).await?;
            while tokio::time::Instant::now() < deadline {
                let id = i32::try_from(ids.fetch_add(1, Ordering::Relaxed))?;
                match crud_cycle(&mut client, id).await {
                    Ok(()) => {
                        operations.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        return Err(error.context(format!("CRUD cycle {id}")));
                    }
                }
            }
            Result::<()>::Ok(())
        });
    }

    let mut samples = Vec::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    while tokio::time::Instant::now() < deadline {
        ticker.tick().await;
        samples.push(resource::sample(proxy_pid, started_at)?);
    }
    while let Some(result) = workers.join_next().await {
        result.context("soak worker panicked")??;
    }
    samples.push(resource::sample(proxy_pid, started_at)?);

    let report = Report {
        duration_seconds: config.duration.as_secs(),
        operations: operations.load(Ordering::Relaxed),
        errors: errors.load(Ordering::Relaxed),
        proxy_pid,
        initial_rss_bytes: samples.first().map_or(0, |sample| sample.rss_bytes),
        final_rss_bytes: samples.last().map_or(0, |sample| sample.rss_bytes),
        peak_rss_bytes: resource::peak_bytes(&samples),
        rss_growth_bytes: resource::growth_bytes(&samples),
        memory_samples: samples,
    };
    if let Some(parent) = config
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&config.output, serde_json::to_vec_pretty(&report)?).await?;
    println!(
        "soak passed: {} CRUD cycles, peak RSS {} MiB, RSS growth {} MiB; report: {}",
        report.operations,
        report.peak_rss_bytes / 1_048_576,
        report.rss_growth_bytes / 1_048_576,
        config.output.display()
    );
    anyhow::ensure!(report.errors == 0, "soak observed {} errors", report.errors);
    if let Some(limit) = config.max_rss_growth_bytes {
        anyhow::ensure!(
            report.rss_growth_bytes <= limit,
            "proxy RSS grew by {} bytes, above the {} byte limit",
            report.rss_growth_bytes,
            limit
        );
    }
    Ok(())
}

async fn crud_cycle(client: &mut tokio_postgres::Client, id: i32) -> Result<()> {
    let transaction = client.transaction().await?;
    let name = format!("soak-customer-{id}");
    let sku = format!("SOAK-{id}");
    transaction
        .execute(
            "INSERT INTO burnin_commerce_customers (id, name) VALUES ($1, $2)",
            &[&id, &name],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO burnin_commerce_products (id, sku, price_cents) VALUES ($1, $2, $3)",
            &[&id, &sku, &(100 + id % 10_000)],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO burnin_commerce_orders (id, customer_id, status) VALUES ($1, $1, 'open')",
            &[&id],
        )
        .await?;
    transaction.execute(
        "INSERT INTO burnin_commerce_order_lines (order_id, line_number, product_id, quantity) VALUES ($1, 1, $1, 2)", &[&id]
    ).await?;
    let row = transaction
        .query_one(
            "SELECT c.name, p.sku, p.price_cents * l.quantity \
         FROM burnin_commerce_orders o \
         JOIN burnin_commerce_customers c ON c.id = o.customer_id \
         JOIN burnin_commerce_order_lines l ON l.order_id = o.id \
         JOIN burnin_commerce_products p ON p.id = l.product_id WHERE o.id = $1",
            &[&id],
        )
        .await?;
    anyhow::ensure!(
        row.get::<_, String>(0) == name && row.get::<_, String>(1) == sku,
        "read-after-write mismatch"
    );
    transaction
        .execute(
            "UPDATE burnin_commerce_orders SET status = 'fulfilled' WHERE id = $1",
            &[&id],
        )
        .await?;
    transaction
        .execute(
            "DELETE FROM burnin_commerce_order_lines WHERE order_id = $1",
            &[&id],
        )
        .await?;
    transaction
        .execute("DELETE FROM burnin_commerce_orders WHERE id = $1", &[&id])
        .await?;
    transaction
        .execute("DELETE FROM burnin_commerce_products WHERE id = $1", &[&id])
        .await?;
    transaction
        .execute(
            "DELETE FROM burnin_commerce_customers WHERE id = $1",
            &[&id],
        )
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn build_release_proxy() -> Result<()> {
    let status = Command::new("cargo")
        .args([
            "build",
            "--locked",
            "--release",
            "--package",
            "cipherstash-proxy",
        ])
        .current_dir(workspace_root())
        .status()
        .await
        .context("building release proxy")?;
    anyhow::ensure!(status.success(), "release proxy build failed with {status}");
    Ok(())
}

fn spawn_release_proxy() -> Result<Child> {
    let binary = workspace_root().join("target/release/cipherstash-proxy");
    anyhow::ensure!(
        binary.is_file(),
        "release proxy binary is missing at {}",
        binary.display()
    );
    Command::new(binary)
        .current_dir(workspace_root())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("starting release proxy")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("burn-in package must be under workspace packages")
}
