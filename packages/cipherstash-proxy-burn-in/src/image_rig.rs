//! RSS growth measurement of a prebuilt Proxy image.
//!
//! Unlike [`crate::soak`], this rig never builds Proxy. It runs a released
//! image unmodified in a Linux container, so the measured process uses the
//! same binary and the same glibc allocator as production. PostgreSQL runs
//! beside it with TLS required, and Proxy verifies the server certificate
//! against a throwaway CA installed through the image's own certificate-bundle
//! entrypoint path.
//!
//! The idle scenario opens zero client connections. The only activity is
//! Proxy's own periodic schema and encrypt-config reloads, so any growth is
//! time-driven.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::{io::AsyncWriteExt, process::Command, task::JoinSet};

use crate::{
    slope,
    soak::{preflight_output, workspace_root, write_report_atomic},
};

/// Reload cadence Proxy uses when neither reload interval is configured.
pub const DEFAULT_RELOAD_INTERVAL_SECONDS: u64 = 60;

const DATABASE_NAME: &str = "cipherstash";
const DATABASE_USER: &str = "cipherstash";
/// Throwaway password for a PostgreSQL container that publishes no host port.
const DATABASE_PASSWORD: &str = "burn-in-rss-rig";
/// The kernel truncates `/proc/<pid>/comm` to 15 bytes.
const PROXY_COMM: &str = "cipherstash-pro";
const PG_HBA: &str = "local all all trust\nhostssl all all all scram-sha-256\n";
const REQUIRED_CREDENTIALS: [&str; 5] = [
    "CS_WORKSPACE_CRN",
    "CS_CLIENT_ACCESS_KEY",
    "CS_DEFAULT_KEYSET_ID",
    "CS_CLIENT_ID",
    "CS_CLIENT_KEY",
];
const OPTIONAL_PASSTHROUGH: [&str; 2] = ["CS_CTS_HOST", "CS_ZEROKMS_HOST"];
const POSTGRES_READY_TIMEOUT: Duration = Duration::from_secs(90);
const PROXY_START_TIMEOUT: Duration = Duration::from_secs(120);
const SECONDS_PER_DAY: f64 = 86_400.0;
const MIB: f64 = 1_048_576.0;

#[derive(Debug, Clone)]
pub struct Config {
    pub image: String,
    pub postgres_image: String,
    pub duration: Duration,
    pub warmup: Duration,
    pub sample_interval: Duration,
    pub reload_intervals: Vec<u64>,
    pub memory_limit: String,
    pub proxy_env: Vec<(String, String)>,
    pub output: PathBuf,
    pub keep_containers: bool,
}

#[derive(Debug, Serialize)]
struct Report {
    status: &'static str,
    scenario: &'static str,
    client_connections: u32,
    started_at_unix_seconds: u64,
    requested_duration_seconds: u64,
    warmup_seconds: u64,
    sample_interval_seconds: u64,
    image: ImageIdentity,
    postgres_image: String,
    docker_server: String,
    memory_limit: String,
    tls: TlsEvidence,
    extra_proxy_env: Vec<String>,
    runs: Vec<RunReport>,
}

#[derive(Debug, Clone, Serialize)]
struct ImageIdentity {
    reference: String,
    digest: String,
    architecture: String,
    proxy_version: String,
}

#[derive(Debug, Serialize)]
struct TlsEvidence {
    postgres_ssl: bool,
    /// The only TCP rule is `hostssl`, so PostgreSQL rejects plaintext TCP.
    pg_hba: &'static str,
    proxy_with_tls_verification: bool,
    ca_trust: &'static str,
}

#[derive(Debug, Serialize)]
struct RunReport {
    label: String,
    reload_interval_seconds: u64,
    status: &'static str,
    terminal_error: Option<String>,
    proxy_container: Option<ContainerState>,
    elapsed_seconds: f64,
    fit_window_seconds: f64,
    postgres_connections: Connections,
    rss: Option<Growth>,
    anonymous: Option<Growth>,
    samples: Vec<Sample>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ContainerState {
    running: bool,
    oom_killed: bool,
    exit_code: i64,
}

/// Connections PostgreSQL authorised, counted from its `log_connections` output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
struct Connections {
    tls_total: u64,
    /// Unix-socket connections made by the PostgreSQL image's own init scripts.
    non_tls_total: u64,
    tls_in_fit_window: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sample {
    pub elapsed_millis: u128,
    pub rss_bytes: u64,
    pub anonymous_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Growth {
    first_bytes: u64,
    last_bytes: u64,
    fitted_samples: usize,
    slope_bytes_per_second: f64,
    r_squared: Option<f64>,
    /// Growth per reload interval, in which each of the schema and
    /// encrypt-config reloaders fires once.
    bytes_per_reload: f64,
    /// Growth per PostgreSQL connection observed in the fit window.
    bytes_per_connection: Option<f64>,
    /// The fitted slope of this run, extrapolated to a day.
    observed_mib_per_day: f64,
    /// `bytes_per_reload` at the default 60s reload cadence. Equal to
    /// `observed_mib_per_day` only if growth scales with reload count.
    projected_mib_per_day_at_default_intervals: f64,
}

pub async fn run(config: Config) -> Result<()> {
    validate(&config)?;
    preflight_output(&config.output).await?;
    let docker_server = docker(&[
        "version",
        "--format",
        "{{.Server.Version}} {{.Server.Os}}/{{.Server.Arch}}",
    ])
    .await
    .context("Docker daemon is not reachable")?;
    let image = identify_image(&config.image).await?;
    ensure_image(&config.postgres_image).await?;
    println!(
        "measuring {} ({}, {}, Proxy {})",
        image.reference, image.digest, image.architecture, image.proxy_version
    );

    let started_at_unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let work_dir = config
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        .join("rss-rig");

    // Each reload setting gets its own PostgreSQL and Proxy containers, so the
    // runs share wall-clock time without sharing connection counts.
    let mut runs = JoinSet::new();
    for interval in &config.reload_intervals {
        let rig = Rig::new(&config, *interval, &work_dir);
        runs.spawn(rig.run());
    }
    let mut reports = Vec::new();
    while let Some(report) = runs.join_next().await {
        reports.push(report.context("RSS rig task panicked")?);
    }
    reports.sort_by_key(|report| std::cmp::Reverse(report.reload_interval_seconds));

    let completed = reports.iter().all(|report| report.status == "completed");
    let report = Report {
        status: if completed { "completed" } else { "failed" },
        scenario: "idle",
        client_connections: 0,
        started_at_unix_seconds,
        requested_duration_seconds: config.duration.as_secs(),
        warmup_seconds: config.warmup.as_secs(),
        sample_interval_seconds: config.sample_interval.as_secs(),
        image,
        postgres_image: config.postgres_image.clone(),
        docker_server,
        memory_limit: config.memory_limit.clone(),
        tls: TlsEvidence {
            postgres_ssl: true,
            pg_hba: PG_HBA,
            proxy_with_tls_verification: true,
            ca_trust: "throwaway CA installed by the image entrypoint via CS_DATABASE__INSTALL_AWS_RDS_CERT_BUNDLE",
        },
        extra_proxy_env: config
            .proxy_env
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect(),
        runs: reports,
    };
    write_report_atomic(&config.output, &report).await?;
    print_summary(&report, &config.output);
    anyhow::ensure!(
        completed,
        "at least one run did not complete; see {}",
        config.output.display()
    );
    Ok(())
}

fn validate(config: &Config) -> Result<()> {
    anyhow::ensure!(
        !config.sample_interval.is_zero(),
        "--sample-interval-seconds must be positive"
    );
    anyhow::ensure!(
        config.duration >= config.warmup + config.sample_interval * 6,
        "--duration-seconds must leave at least six samples after --warmup-seconds"
    );
    anyhow::ensure!(
        !config.reload_intervals.is_empty() && config.reload_intervals.iter().all(|i| *i > 0),
        "--reload-interval-seconds must be positive"
    );
    let mut unique = config.reload_intervals.clone();
    unique.sort_unstable();
    unique.dedup();
    anyhow::ensure!(
        unique.len() == config.reload_intervals.len(),
        "--reload-interval-seconds values must be distinct"
    );
    for name in REQUIRED_CREDENTIALS {
        anyhow::ensure!(
            std::env::var(name).is_ok_and(|value| !value.is_empty()),
            "{name} must be set so Proxy can reach ZeroKMS"
        );
    }
    Ok(())
}

async fn ensure_image(image: &str) -> Result<()> {
    if docker(&["image", "inspect", "--format", "{{.Id}}", image])
        .await
        .is_err()
    {
        docker(&["pull", "--quiet", image])
            .await
            .with_context(|| format!("pulling {image}"))?;
    }
    Ok(())
}

async fn identify_image(image: &str) -> Result<ImageIdentity> {
    ensure_image(image).await?;
    let described = docker(&[
        "image",
        "inspect",
        "--format",
        "{{.Architecture}} {{if .RepoDigests}}{{index .RepoDigests 0}}{{else}}{{.Id}}{{end}}",
        image,
    ])
    .await?;
    let (architecture, digest) = described
        .split_once(' ')
        .context("unexpected docker image inspect output")?;
    let proxy_version = docker(&[
        "run",
        "--rm",
        "--entrypoint",
        "cipherstash-proxy",
        image,
        "--version",
    ])
    .await
    .context("reading the Proxy version from the image")?;
    Ok(ImageIdentity {
        reference: image.to_string(),
        digest: digest.to_string(),
        architecture: architecture.to_string(),
        proxy_version,
    })
}

/// One reload setting: its containers, network and working files.
struct Rig {
    config: Config,
    reload_interval: u64,
    label: String,
    network: String,
    postgres: String,
    proxy: String,
    dir: PathBuf,
}

impl Rig {
    fn new(config: &Config, reload_interval: u64, work_dir: &Path) -> Self {
        let label = format!("reload-{reload_interval}s");
        let prefix = format!("burn-in-rss-{label}");
        Self {
            config: config.clone(),
            reload_interval,
            network: prefix.clone(),
            postgres: format!("{prefix}-postgres"),
            proxy: format!("{prefix}-proxy"),
            dir: work_dir.join(&label),
            label,
        }
    }

    async fn run(self) -> RunReport {
        let mut report = RunReport {
            label: self.label.clone(),
            reload_interval_seconds: self.reload_interval,
            status: "failed",
            terminal_error: None,
            proxy_container: None,
            elapsed_seconds: 0.0,
            fit_window_seconds: 0.0,
            postgres_connections: Connections::default(),
            rss: None,
            anonymous: None,
            samples: Vec::new(),
        };
        match self.measure(&mut report).await {
            Ok(status) => report.status = status,
            Err(error) => report.terminal_error = Some(format!("{error:#}")),
        }
        report.proxy_container = self.container_state().await.ok();
        self.save_logs().await;
        if report.status != "completed" {
            if let Ok(logs) = docker_combined(&["logs", "--tail", "40", &self.proxy]).await {
                eprintln!("[{}] last Proxy log lines:\n{logs}", self.label);
            }
        }
        if !self.config.keep_containers {
            self.remove().await;
        }
        self.summarise(&mut report);
        report
    }

    async fn measure(&self, report: &mut RunReport) -> Result<&'static str> {
        self.remove().await;
        self.write_tls_material().await?;
        docker(&["network", "create", &self.network]).await?;
        self.start_postgres().await?;
        self.start_proxy().await?;
        self.wait_for_proxy_process().await?;
        println!("[{}] Proxy started; sampling", self.label);

        let mut series = tokio::fs::File::create(self.dir.join("samples.jsonl")).await?;
        let started_at = Instant::now();
        let mut ticker = tokio::time::interval(self.config.sample_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut connections_at_warmup = None;
        let mut next_progress = Duration::ZERO;
        let status = loop {
            tokio::select! {
                _ = ticker.tick() => {}
                signal = tokio::signal::ctrl_c() => {
                    signal.context("installing interrupt handler")?;
                    break "interrupted";
                }
            }
            let elapsed = started_at.elapsed();
            let sample = match self.sample(started_at).await {
                Ok(sample) => sample,
                Err(error) => {
                    report.terminal_error = Some(format!("{error:#}"));
                    break "proxy_exited";
                }
            };
            series
                .write_all(format!("{}\n", serde_json::to_string(&sample)?).as_bytes())
                .await?;
            if elapsed >= next_progress {
                println!(
                    "[{}] {:>6}s rss {:.2} MiB anonymous {:.2} MiB",
                    self.label,
                    elapsed.as_secs(),
                    sample.rss_bytes as f64 / MIB,
                    sample.anonymous_bytes as f64 / MIB
                );
                next_progress = elapsed + Duration::from_secs(300);
            }
            report.samples.push(sample);
            if connections_at_warmup.is_none() && elapsed >= self.config.warmup {
                connections_at_warmup = Some(self.count_connections().await?);
            }
            if elapsed >= self.config.duration {
                break "completed";
            }
        };
        series.flush().await?;

        let total = self.count_connections().await?;
        report.postgres_connections = Connections {
            tls_in_fit_window: connections_at_warmup
                .map_or(0, |warm| total.tls_total.saturating_sub(warm.tls_total)),
            ..total
        };
        if status == "completed" {
            anyhow::ensure!(
                report.postgres_connections.tls_in_fit_window > 0,
                "Proxy opened no TLS connections to PostgreSQL after warm-up, so its reloaders did not run"
            );
        }
        Ok(status)
    }

    fn summarise(&self, report: &mut RunReport) {
        let warmup_millis = self.config.warmup.as_millis();
        let fitted: Vec<&Sample> = report
            .samples
            .iter()
            .filter(|sample| sample.elapsed_millis >= warmup_millis)
            .collect();
        report.elapsed_seconds = report
            .samples
            .last()
            .map_or(0.0, |sample| sample.elapsed_millis as f64 / 1000.0);
        let (Some(first), Some(last)) = (fitted.first(), fitted.last()) else {
            return;
        };
        report.fit_window_seconds = (last.elapsed_millis - first.elapsed_millis) as f64 / 1000.0;
        let connections = report.postgres_connections.tls_in_fit_window;
        report.rss = growth(
            &fitted,
            |sample| sample.rss_bytes,
            self.reload_interval,
            report.fit_window_seconds,
            connections,
        );
        report.anonymous = growth(
            &fitted,
            |sample| sample.anonymous_bytes,
            self.reload_interval,
            report.fit_window_seconds,
            connections,
        );
    }

    /// Generates a throwaway CA and a server certificate for the PostgreSQL
    /// container's network name, which is the host Proxy verifies against.
    async fn write_tls_material(&self) -> Result<()> {
        let _ = tokio::fs::remove_dir_all(&self.dir).await;
        let postgres_dir = self.dir.join("postgres");
        let ca_dir = self.dir.join("ca");
        tokio::fs::create_dir_all(&postgres_dir).await?;
        tokio::fs::create_dir_all(&ca_dir).await?;
        tokio::fs::write(
            self.dir.join("ca.cnf"),
            "[req]\ndistinguished_name=dn\nx509_extensions=v3_ca\nprompt=no\n\
             [dn]\nCN=CipherStash burn-in RSS rig CA\n\
             [v3_ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n",
        )
        .await?;
        tokio::fs::write(
            self.dir.join("server.ext"),
            format!(
                "subjectAltName=DNS:{}\nbasicConstraints=CA:FALSE\n\
                 keyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n",
                self.postgres
            ),
        )
        .await?;
        tokio::fs::write(postgres_dir.join("pg_hba.conf"), PG_HBA).await?;

        let subject = format!("/CN={}", self.postgres);
        self.openssl(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "30",
            "-config",
            "ca.cnf",
            "-keyout",
            "ca.key",
            "-out",
            "ca/ca.pem",
        ])
        .await?;
        self.openssl(&[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            &subject,
            "-keyout",
            "postgres/server.key",
            "-out",
            "server.csr",
        ])
        .await?;
        self.openssl(&[
            "x509",
            "-req",
            "-in",
            "server.csr",
            "-CA",
            "ca/ca.pem",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-days",
            "30",
            "-extfile",
            "server.ext",
            "-out",
            "postgres/server.pem",
        ])
        .await?;
        Ok(())
    }

    async fn openssl(&self, args: &[&str]) -> Result<()> {
        let output = Command::new("openssl")
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .output()
            .await
            .context("running openssl; it must be installed to generate the rig certificates")?;
        anyhow::ensure!(
            output.status.success(),
            "openssl {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }

    async fn start_postgres(&self) -> Result<()> {
        let mount = format!("{}:/rig:ro", absolute(&self.dir.join("postgres"))?);
        // The key must be owned by the postgres user, which a read-only bind
        // mount cannot guarantee, so the files are copied inside the container.
        let command = "mkdir /tls && cp /rig/server.pem /rig/server.key /rig/pg_hba.conf /tls/ \
             && chown -R postgres:postgres /tls && chmod 0600 /tls/server.key \
             && exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tls/server.pem \
             -c ssl_key_file=/tls/server.key -c hba_file=/tls/pg_hba.conf -c log_connections=on";
        docker(&[
            "run",
            "--detach",
            "--name",
            &self.postgres,
            "--network",
            &self.network,
            "--volume",
            &mount,
            "--env",
            &format!("POSTGRES_DB={DATABASE_NAME}"),
            "--env",
            &format!("POSTGRES_USER={DATABASE_USER}"),
            "--env",
            &format!("POSTGRES_PASSWORD={DATABASE_PASSWORD}"),
            &self.config.postgres_image,
            "sh",
            "-c",
            command,
        ])
        .await?;

        // The image's init phase listens on the Unix socket only, so a TCP
        // check passes only once the final server is up.
        let deadline = Instant::now() + POSTGRES_READY_TIMEOUT;
        loop {
            let ready = docker(&[
                "exec",
                &self.postgres,
                "pg_isready",
                "--host",
                "127.0.0.1",
                "--username",
                DATABASE_USER,
                "--dbname",
                DATABASE_NAME,
            ])
            .await;
            if ready.is_ok() {
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "PostgreSQL container {} did not become ready",
                self.postgres
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn start_proxy(&self) -> Result<()> {
        let mount = format!("{}:/rig:ro", absolute(&self.dir.join("ca"))?);
        let interval = self.reload_interval.to_string();
        let mut env: Vec<(String, String)> = [
            ("CS_DATABASE__HOST", self.postgres.as_str()),
            ("CS_DATABASE__PORT", "5432"),
            ("CS_DATABASE__NAME", DATABASE_NAME),
            ("CS_DATABASE__USERNAME", DATABASE_USER),
            ("CS_DATABASE__PASSWORD", DATABASE_PASSWORD),
            ("CS_DATABASE__WITH_TLS_VERIFICATION", "true"),
            // Proxy 2.2.5 drives both reloaders from the config reload
            // interval. Setting both keeps the rig correct for either reading.
            ("CS_DATABASE__CONFIG_RELOAD_INTERVAL", interval.as_str()),
            ("CS_DATABASE__SCHEMA_RELOAD_INTERVAL", interval.as_str()),
            // Stock entrypoint behaviour: trust the rig CA, install the EQL
            // bundled in the image, and create its example encrypted table.
            ("CS_DATABASE__INSTALL_AWS_RDS_CERT_BUNDLE", "true"),
            ("CS_DATABASE__AWS_BUNDLE_PATH", "/rig/ca.pem"),
            ("CS_DATABASE__INSTALL_EQL", "true"),
            ("CS_DATABASE__INSTALL_EXAMPLE_SCHEMA", "true"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
        env.extend(self.config.proxy_env.iter().cloned());

        let mut args: Vec<String> = [
            "run",
            "--detach",
            "--name",
            &self.proxy,
            "--network",
            &self.network,
            "--memory",
            &self.config.memory_limit,
            "--memory-swap",
            &self.config.memory_limit,
            "--volume",
            &mount,
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        for (name, value) in env {
            args.extend(["--env".to_string(), format!("{name}={value}")]);
        }
        // Credentials are inherited by name so they never appear in argv.
        let optional = OPTIONAL_PASSTHROUGH
            .into_iter()
            .filter(|name| std::env::var(name).is_ok_and(|value| !value.is_empty()));
        for name in REQUIRED_CREDENTIALS.into_iter().chain(optional) {
            args.extend(["--env".to_string(), name.to_string()]);
        }
        args.push(self.config.image.clone());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        docker(&args).await?;
        Ok(())
    }

    /// Waits for the entrypoint to `exec` Proxy as PID 1.
    ///
    /// No client connection is opened to check readiness, because the idle
    /// scenario must have none. A Proxy that fails to initialise exits, which
    /// the sampling loop reports.
    async fn wait_for_proxy_process(&self) -> Result<()> {
        let deadline = Instant::now() + PROXY_START_TIMEOUT;
        loop {
            if let Ok(comm) = docker(&["exec", &self.proxy, "cat", "/proc/1/comm"]).await {
                if comm == PROXY_COMM {
                    return Ok(());
                }
            }
            let state = self.container_state().await?;
            anyhow::ensure!(
                state.running,
                "Proxy container exited during startup with code {}",
                state.exit_code
            );
            anyhow::ensure!(
                Instant::now() < deadline,
                "the image entrypoint did not start Proxy within {} seconds",
                PROXY_START_TIMEOUT.as_secs()
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn sample(&self, started_at: Instant) -> Result<Sample> {
        let rollup = docker(&[
            "exec",
            &self.proxy,
            "sh",
            "-c",
            "cat /proc/1/comm /proc/1/smaps_rollup",
        ])
        .await
        .context("Proxy process is no longer running")?;
        let elapsed_millis = started_at.elapsed().as_millis();
        anyhow::ensure!(
            rollup.lines().next() == Some(PROXY_COMM),
            "PID 1 in the Proxy container is not cipherstash-proxy"
        );
        let (rss_bytes, anonymous_bytes) = parse_smaps_rollup(&rollup)?;
        Ok(Sample {
            elapsed_millis,
            rss_bytes,
            anonymous_bytes,
        })
    }

    async fn count_connections(&self) -> Result<Connections> {
        let logs = docker_combined(&["logs", &self.postgres]).await?;
        Ok(count_authorized_connections(&logs))
    }

    async fn container_state(&self) -> Result<ContainerState> {
        let state = docker(&[
            "inspect",
            "--format",
            "{{.State.Running}} {{.State.OOMKilled}} {{.State.ExitCode}}",
            &self.proxy,
        ])
        .await?;
        let mut fields = state.split_whitespace();
        Ok(ContainerState {
            running: fields.next() == Some("true"),
            oom_killed: fields.next() == Some("true"),
            exit_code: fields.next().unwrap_or_default().parse()?,
        })
    }

    async fn save_logs(&self) {
        for (container, file) in [(&self.proxy, "proxy.log"), (&self.postgres, "postgres.log")] {
            if let Ok(logs) = docker_combined(&["logs", container]).await {
                let _ = tokio::fs::write(self.dir.join(file), logs).await;
            }
        }
    }

    async fn remove(&self) {
        let _ = docker(&["rm", "--force", "--volumes", &self.proxy, &self.postgres]).await;
        let _ = docker(&["network", "rm", &self.network]).await;
    }
}

fn growth(
    samples: &[&Sample],
    value: impl Fn(&Sample) -> u64,
    reload_interval: u64,
    fit_window_seconds: f64,
    connections: u64,
) -> Option<Growth> {
    let points: Vec<(f64, f64)> = samples
        .iter()
        .map(|sample| (sample.elapsed_millis as f64 / 1000.0, value(sample) as f64))
        .collect();
    let fit = slope::fit(&points)?;
    let bytes_per_reload = fit.slope_per_second * reload_interval as f64;
    Some(Growth {
        first_bytes: value(samples.first()?),
        last_bytes: value(samples.last()?),
        fitted_samples: fit.samples,
        slope_bytes_per_second: fit.slope_per_second,
        r_squared: fit.r_squared,
        bytes_per_reload,
        bytes_per_connection: (connections > 0)
            .then(|| fit.slope_per_second * fit_window_seconds / connections as f64),
        observed_mib_per_day: fit.slope_per_second * SECONDS_PER_DAY / MIB,
        projected_mib_per_day_at_default_intervals: bytes_per_reload
            * (SECONDS_PER_DAY / DEFAULT_RELOAD_INTERVAL_SECONDS as f64)
            / MIB,
    })
}

/// Extracts total RSS and anonymous memory, in bytes, from `smaps_rollup`.
pub fn parse_smaps_rollup(rollup: &str) -> Result<(u64, u64)> {
    let field = |name: &str| -> Result<u64> {
        let kib = rollup
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .with_context(|| format!("{name} was absent from smaps_rollup"))?
            .parse::<u64>()?;
        Ok(kib * 1024)
    };
    Ok((field("Rss:")?, field("Anonymous:")?))
}

fn count_authorized_connections(postgres_logs: &str) -> Connections {
    let mut connections = Connections::default();
    for line in postgres_logs
        .lines()
        .filter(|line| line.contains("connection authorized:"))
    {
        if line.contains("SSL enabled") {
            connections.tls_total += 1;
        } else {
            connections.non_tls_total += 1;
        }
    }
    connections
}

fn absolute(path: &Path) -> Result<String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    path.to_str()
        .map(str::to_string)
        .context("rig working directory is not valid UTF-8")
}

async fn docker(args: &[&str]) -> Result<String> {
    let output = docker_output(args).await?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Container logs arrive on both streams; PostgreSQL logs to stderr.
async fn docker_combined(args: &[&str]) -> Result<String> {
    let output = docker_output(args).await?;
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(combined)
}

async fn docker_output(args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new("docker")
        .args(args)
        .current_dir(workspace_root())
        .stdin(Stdio::null())
        .output()
        .await
        .context("running docker")?;
    anyhow::ensure!(
        output.status.success(),
        "docker {} failed: {}",
        args.first().copied().unwrap_or_default(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output)
}

fn print_summary(report: &Report, output: &Path) {
    println!(
        "\nidle RSS growth of {} (Proxy {}), TLS verified, zero client connections",
        report.image.reference, report.image.proxy_version
    );
    for run in &report.runs {
        println!(
            "[{}] {} after {:.0}s, {} TLS connections in the {:.0}s fit window",
            run.label,
            run.status,
            run.elapsed_seconds,
            run.postgres_connections.tls_in_fit_window,
            run.fit_window_seconds
        );
        for (name, growth) in [("rss", &run.rss), ("anonymous", &run.anonymous)] {
            let Some(growth) = growth else { continue };
            println!(
                "  {name:<9} {:>10.1} bytes/reload  {:>8.2} MiB/day observed  {:>8.2} MiB/day at 60s reloads  R² {}",
                growth.bytes_per_reload,
                growth.observed_mib_per_day,
                growth.projected_mib_per_day_at_default_intervals,
                growth
                    .r_squared
                    .map_or_else(|| "n/a".to_string(), |value| format!("{value:.3}"))
            );
        }
        if let Some(error) = &run.terminal_error {
            println!("  error: {error}");
        }
    }
    println!("report: {}", output.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLLUP: &str = "cipherstash-pro
aaaac0000000-ffffd0000000 ---p 00000000 00:00 0                          [rollup]
Rss:               41280 kB
Pss:               40112 kB
Pss_Anon:          21504 kB
Shared_Clean:       1200 kB
Anonymous:         21504 kB
AnonHugePages:         0 kB
";

    #[test]
    fn parses_rss_and_anonymous_from_smaps_rollup() {
        assert_eq!(
            parse_smaps_rollup(ROLLUP).unwrap(),
            (41_280 * 1024, 21_504 * 1024)
        );
    }

    #[test]
    fn rejects_rollup_without_anonymous() {
        assert!(parse_smaps_rollup("Rss: 10 kB\n").is_err());
    }

    #[test]
    fn counts_tls_and_socket_connections_separately() {
        let logs = "\
LOG:  connection received: host=172.18.0.3 port=40012
LOG:  connection authenticated: identity=\"cipherstash\" method=scram-sha-256
LOG:  connection authorized: user=cipherstash database=cipherstash SSL enabled (protocol=TLSv1.3, cipher=TLS_AES_256_GCM_SHA384, bits=256)
LOG:  connection authorized: user=cipherstash database=cipherstash application_name=psql
LOG:  connection authorized: user=cipherstash database=cipherstash SSL enabled (protocol=TLSv1.3, cipher=TLS_AES_256_GCM_SHA384, bits=256)
";
        assert_eq!(
            count_authorized_connections(logs),
            Connections {
                tls_total: 2,
                non_tls_total: 1,
                tls_in_fit_window: 0,
            }
        );
    }

    #[test]
    fn growth_converts_slope_to_reload_and_daily_rates() {
        // 1 KiB/s for 100 seconds at a 1s reload interval, 200 connections.
        let samples: Vec<Sample> = (0..=10)
            .map(|step| Sample {
                elapsed_millis: step * 10_000,
                rss_bytes: 1_000_000 + u64::try_from(step).unwrap() * 10_240,
                anonymous_bytes: 0,
            })
            .collect();
        let fitted: Vec<&Sample> = samples.iter().collect();
        let growth = growth(&fitted, |sample| sample.rss_bytes, 1, 100.0, 200).unwrap();
        assert!((growth.slope_bytes_per_second - 1024.0).abs() < 1e-6);
        assert!((growth.bytes_per_reload - 1024.0).abs() < 1e-6);
        assert!((growth.bytes_per_connection.unwrap() - 512.0).abs() < 1e-6);
        assert!((growth.observed_mib_per_day - 84.375).abs() < 1e-6);
        // 1024 bytes per reload at 1,440 reloads per day.
        assert!((growth.projected_mib_per_day_at_default_intervals - 1.40625).abs() < 1e-6);
    }
}
