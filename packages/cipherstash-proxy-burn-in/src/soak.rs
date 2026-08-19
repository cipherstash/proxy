//! Timed encrypted CRUD workload with release-Proxy RSS sampling.
//!
//! This module builds and owns the exact Proxy process being measured. Every
//! CRUD cycle writes and reads EQL-domain columns; fixture setup also verifies
//! ciphertext directly in PostgreSQL before timing begins. Memory growth
//! therefore includes the encryption/decryption path rather than passthrough
//! SQL alone.

use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{
    process::{Child, Command},
    task::JoinSet,
    time::timeout,
};

use crate::{
    database::{self, DatabaseTarget},
    resource::{self, MemorySample},
};

#[derive(Debug)]
pub struct Config {
    pub duration: Duration,
    pub concurrency: usize,
    pub proxy_database_url: DatabaseTarget,
    pub direct_database_url: DatabaseTarget,
    pub eql_path: PathBuf,
    pub output: PathBuf,
    pub max_rss_growth_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
struct Report {
    status: &'static str,
    terminal_error: Option<String>,
    requested_duration_seconds: u64,
    actual_elapsed_millis: u128,
    concurrency: usize,
    started_at_unix_seconds: u64,
    artifact: String,
    source_commit: String,
    database: String,
    operations: u64,
    errors: u64,
    proxy_pid: u32,
    initial_rss_bytes: u64,
    final_rss_bytes: u64,
    peak_rss_bytes: u64,
    rss_growth_bytes: u64,
    memory_samples: Vec<MemorySample>,
}

const READY_TIMEOUT: Duration = Duration::from_secs(30);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn run(config: Config) -> Result<()> {
    anyhow::ensure!(
        !config.duration.is_zero(),
        "--duration-seconds must be positive"
    );
    anyhow::ensure!(config.concurrency > 0, "--concurrency must be positive");
    preflight_output(&config.output).await?;
    let _run_lock = database::acquire_run_lock(&config.direct_database_url).await?;
    database::ensure_eql_installed(&config.direct_database_url, &config.eql_path).await?;

    let artifact = build_release_proxy().await?;
    preflight_listener(&config.proxy_database_url)?;
    let mut proxy = spawn_release_proxy(
        &artifact,
        &config.proxy_database_url,
        &config.direct_database_url,
    )?;
    let proxy_pid = proxy.id().context("release proxy did not expose a PID")?;
    let started_at = Instant::now();
    let mut report = Report {
        status: "failed",
        terminal_error: None,
        requested_duration_seconds: config.duration.as_secs(),
        actual_elapsed_millis: 0,
        concurrency: config.concurrency,
        started_at_unix_seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        artifact: artifact.display().to_string(),
        source_commit: source_commit(),
        database: config.direct_database_url.to_string(),
        operations: 0,
        errors: 0,
        proxy_pid,
        initial_rss_bytes: 0,
        final_rss_bytes: 0,
        peak_rss_bytes: 0,
        rss_growth_bytes: 0,
        memory_samples: Vec::new(),
    };
    let mut result = run_with_proxy(&config, &mut proxy, &mut report).await;
    let _ = proxy.kill().await;
    let _ = proxy.wait().await;
    report.actual_elapsed_millis = started_at.elapsed().as_millis();
    refresh_rss_summary(&mut report);

    if result.is_ok() {
        result = validate_report(&report, config.max_rss_growth_bytes);
    }
    match &result {
        Ok(()) => report.status = "passed",
        Err(error) => report.terminal_error = Some(format!("{error:#}")),
    }
    write_report_atomic(&config.output, &report).await?;
    result?;
    println!(
        "soak passed: {} CRUD cycles, peak RSS {} MiB, RSS growth {} MiB; report: {}",
        report.operations,
        report.peak_rss_bytes / 1_048_576,
        report.rss_growth_bytes / 1_048_576,
        config.output.display()
    );
    Ok(())
}

async fn run_with_proxy(config: &Config, proxy: &mut Child, report: &mut Report) -> Result<()> {
    wait_until_ready(&config.proxy_database_url, proxy).await?;
    ensure_child_running(proxy, "fixture migration")?;
    timeout(
        OPERATION_TIMEOUT,
        database::migrate(&config.proxy_database_url, &config.direct_database_url),
    )
    .await
    .context("fixture migration timed out")??;
    ensure_child_running(proxy, "workload warm-up")?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    ensure_child_running(proxy, "workload start")?;
    let proxy_pid = proxy.id().context("release proxy did not expose a PID")?;
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
            let mut client = timeout(OPERATION_TIMEOUT, database::connect(&url))
                .await
                .context("worker database connection timed out")??;
            while tokio::time::Instant::now() < deadline {
                let id = i32::try_from(ids.fetch_add(1, Ordering::Relaxed))?;
                match timeout(OPERATION_TIMEOUT, crud_cycle(&mut client, id)).await {
                    Err(_) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        return Err(anyhow::anyhow!("CRUD cycle {id} timed out"));
                    }
                    Ok(Ok(())) => {
                        operations.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Err(error)) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        return Err(error.context(format!("CRUD cycle {id}")));
                    }
                }
            }
            Result::<()>::Ok(())
        });
    }

    let mut ticker = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            _ = ticker.tick() => {}
            signal = tokio::signal::ctrl_c() => {
                signal.context("installing interrupt handler")?;
                anyhow::bail!("burn-in interrupted");
            }
        }
        ensure_child_running(proxy, "RSS sampling")?;
        let sample = resource::sample(proxy_pid, started_at)?;
        anyhow::ensure!(sample.rss_bytes > 0, "Proxy RSS sample was zero");
        report.memory_samples.push(sample);
        report.operations = operations.load(Ordering::Relaxed);
        report.errors = errors.load(Ordering::Relaxed);
    }
    ensure_child_running(proxy, "worker shutdown")?;
    let worker_result = timeout(WORKER_SHUTDOWN_TIMEOUT, async {
        while let Some(result) = workers.join_next().await {
            result.context("soak worker panicked")??;
        }
        Result::<()>::Ok(())
    })
    .await;
    if worker_result.is_err() {
        workers.abort_all();
    }
    let worker_result = worker_result.context("workers did not stop within 15 seconds")?;
    let final_sample = resource::sample(proxy_pid, started_at)?;
    anyhow::ensure!(
        final_sample.rss_bytes > 0,
        "final Proxy RSS sample was zero"
    );
    report.memory_samples.push(final_sample);

    report.operations = operations.load(Ordering::Relaxed);
    report.errors = errors.load(Ordering::Relaxed);
    worker_result?;
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

#[derive(Deserialize)]
struct CargoArtifact {
    reason: String,
    target: Option<CargoTarget>,
    executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct CargoTarget {
    name: String,
}

async fn build_release_proxy() -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args([
            "build",
            "--locked",
            "--release",
            "--package",
            "cipherstash-proxy",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(workspace_root())
        .output()
        .await
        .context("building release proxy")?;
    anyhow::ensure!(
        output.status.success(),
        "release proxy build failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    find_proxy_artifact(&output.stdout)
}

fn find_proxy_artifact(messages: &[u8]) -> Result<PathBuf> {
    let mut artifact = None;
    for line in messages.split(|byte| *byte == b'\n') {
        let Ok(message) = serde_json::from_slice::<CargoArtifact>(line) else {
            continue;
        };
        if message.reason == "compiler-artifact"
            && message
                .target
                .is_some_and(|target| target.name == "cipherstash-proxy")
            && message.executable.is_some()
        {
            artifact = message.executable;
        }
    }
    artifact.context("Cargo did not report the release Proxy executable")
}

fn spawn_release_proxy(
    binary: &Path,
    proxy_database: &DatabaseTarget,
    direct_database: &DatabaseTarget,
) -> Result<Child> {
    anyhow::ensure!(
        binary.is_file(),
        "release proxy binary is missing at {}",
        binary.display()
    );
    let host = proxy_bind_host(proxy_database)?;
    let mut command = Command::new(binary);
    direct_database.configure_proxy_upstream(&mut command)?;
    command
        .env("CS_SERVER__HOST", host)
        .env("CS_SERVER__PORT", proxy_database.port()?.to_string())
        .kill_on_drop(true)
        .current_dir(workspace_root())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command.spawn().context("starting release proxy")
}

fn preflight_listener(proxy_database: &DatabaseTarget) -> Result<()> {
    let listener = TcpListener::bind((proxy_bind_host(proxy_database)?, proxy_database.port()?))
        .context("Proxy listen address is already in use")?;
    drop(listener);
    Ok(())
}

fn proxy_bind_host(proxy_database: &DatabaseTarget) -> Result<&'static str> {
    match proxy_database.hostname()? {
        "localhost" | "127.0.0.1" => Ok("127.0.0.1"),
        "::1" => Ok("::1"),
        _ => anyhow::bail!("spawned Proxy must use a loopback listener"),
    }
}

fn ensure_child_running(child: &mut Child, phase: &str) -> Result<()> {
    if let Some(status) = child.try_wait().context("checking release Proxy status")? {
        anyhow::bail!("release Proxy exited during {phase} with {status}");
    }
    Ok(())
}

async fn wait_until_ready(target: &DatabaseTarget, child: &mut Child) -> Result<()> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        ensure_child_running(child, "startup")?;
        let ready = timeout(Duration::from_secs(2), async {
            let client = database::connect(target).await?;
            client.simple_query("SELECT 1").await?;
            Result::<()>::Ok(())
        })
        .await;
        if matches!(ready, Ok(Ok(()))) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("release Proxy did not become ready within 30 seconds");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn validate_report(report: &Report, max_rss_growth_bytes: Option<u64>) -> Result<()> {
    anyhow::ensure!(report.operations > 0, "soak completed zero CRUD cycles");
    anyhow::ensure!(report.errors == 0, "soak observed {} errors", report.errors);
    anyhow::ensure!(
        !report.memory_samples.is_empty() && report.final_rss_bytes > 0,
        "soak did not capture live Proxy RSS"
    );
    if let Some(limit) = max_rss_growth_bytes {
        anyhow::ensure!(
            report.rss_growth_bytes <= limit,
            "proxy RSS grew by {} bytes, above the {} byte limit",
            report.rss_growth_bytes,
            limit
        );
    }
    Ok(())
}

/// Recomputes summary fields even when the workload terminates early, so a
/// failed report retains all useful memory evidence collected before failure.
fn refresh_rss_summary(report: &mut Report) {
    report.initial_rss_bytes = report
        .memory_samples
        .first()
        .map_or(0, |sample| sample.rss_bytes);
    report.final_rss_bytes = report
        .memory_samples
        .last()
        .map_or(0, |sample| sample.rss_bytes);
    report.peak_rss_bytes = resource::peak_bytes(&report.memory_samples);
    report.rss_growth_bytes = resource::growth_bytes(&report.memory_samples);
}

async fn preflight_output(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temporary = temporary_report_path(path);
    tokio::fs::write(&temporary, b"")
        .await
        .context("preflighting burn-in report output")?;
    tokio::fs::remove_file(temporary).await?;
    Ok(())
}

async fn write_report_atomic(path: &Path, report: &Report) -> Result<()> {
    let temporary = temporary_report_path(path);
    tokio::fs::write(&temporary, serde_json::to_vec_pretty(report)?)
        .await
        .context("writing temporary burn-in report")?;
    tokio::fs::rename(&temporary, path)
        .await
        .context("publishing burn-in report atomically")?;
    Ok(())
}

fn temporary_report_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

fn source_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|commit| commit.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("burn-in package must be under workspace packages")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_executable_from_cargo_json() {
        let messages = br#"{"reason":"compiler-artifact","target":{"name":"other"},"executable":"/tmp/other"}
{"reason":"compiler-artifact","target":{"name":"cipherstash-proxy"},"executable":"/custom/target/release/cipherstash-proxy"}
"#;
        assert_eq!(
            find_proxy_artifact(messages).unwrap(),
            PathBuf::from("/custom/target/release/cipherstash-proxy")
        );
    }

    #[test]
    fn report_requires_work_and_live_rss() {
        let report = Report {
            status: "failed",
            terminal_error: None,
            requested_duration_seconds: 1,
            actual_elapsed_millis: 1,
            concurrency: 1,
            started_at_unix_seconds: 0,
            artifact: "proxy".into(),
            source_commit: "commit".into(),
            database: "postgresql://user@localhost:5432/db".into(),
            operations: 0,
            errors: 0,
            proxy_pid: 1,
            initial_rss_bytes: 0,
            final_rss_bytes: 0,
            peak_rss_bytes: 0,
            rss_growth_bytes: 0,
            memory_samples: vec![],
        };
        assert!(validate_report(&report, None).is_err());
    }
}
