//! Client workloads for the released-image RSS rig.
//!
//! Each workload isolates one kind of client activity so that memory growth
//! can be expressed per unit of it: per connect for the three churn workloads
//! and per query for the pooled one.
//!
//! Proxy 2.2.5 opens its PostgreSQL connection as soon as a client's TCP
//! connection is accepted, before it reads the startup message. A bare TCP
//! probe therefore costs Proxy a full TLS connect to the database, which is
//! why the churn workloads stop at different points of the handshake.
//!
//! The pooled workload uses the `users` table that the image's entrypoint
//! installs with `CS_DATABASE__INSTALL_EXAMPLE_SCHEMA`. That table is the EQL
//! v2 fixture: `encrypted_email` and `encrypted_salary` are encrypted columns
//! with unique, match and ORE indexes.

use std::{
    net::SocketAddr,
    str::FromStr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    task::JoinSet,
    time::{timeout, MissedTickBehavior},
};
use tokio_postgres::{Client, NoTls};

/// Bounds one connect or one query, so a stalled Proxy shows up as errors
/// rather than as a workload that silently stops.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
/// PostgreSQL protocol 3.0, the version field of a startup message.
const PROTOCOL_VERSION: i32 = 196_608;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Workload {
    /// Open a TCP connection and close it, as a Kubernetes `tcpSocket` probe does.
    TcpProbe,
    /// Send a startup message, read the authentication request and close, as
    /// `pg_isready` does.
    StartupProbe,
    /// Authenticate fully, then send `Terminate`, as an unpooled client does.
    LoginChurn,
    /// Connections that stay open and idle until Proxy closes them at its
    /// `connection_timeout`, then reconnect, as a pool that keeps idle
    /// connections does.
    IdlePool,
    /// Long-lived connections running an encrypted read and write mix.
    PooledQueries,
}

impl Workload {
    /// The unit growth is expressed per.
    pub fn unit(self) -> &'static str {
        match self {
            Self::TcpProbe | Self::StartupProbe | Self::LoginChurn | Self::IdlePool => "connect",
            Self::PooledQueries => "query",
        }
    }
}

impl FromStr for Workload {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "tcp-probe" => Ok(Self::TcpProbe),
            "startup-probe" => Ok(Self::StartupProbe),
            "login-churn" => Ok(Self::LoginChurn),
            "idle-pool" => Ok(Self::IdlePool),
            "pooled-queries" => Ok(Self::PooledQueries),
            _ => Err(
                "expected tcp-probe, startup-probe, login-churn, idle-pool or pooled-queries"
                    .to_string(),
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub workload: Workload,
    /// Period of each connection's loop: one connect, or one query.
    pub interval: Duration,
    /// Concurrent loops. For the pooled workload this is the pool size.
    pub connections: usize,
    /// Production rate the per-unit growth is projected at. Defaults to the
    /// rate this run achieved.
    pub assumed_operations_per_day: Option<f64>,
}

/// How to reach and authenticate to Proxy from the rig process.
#[derive(Debug, Clone)]
pub struct Target {
    pub address: SocketAddr,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl Target {
    fn postgres_config(&self) -> tokio_postgres::Config {
        let mut config = tokio_postgres::Config::new();
        config
            .host(self.address.ip().to_string())
            .port(self.address.port())
            .user(&self.user)
            .password(&self.password)
            .dbname(&self.database)
            .connect_timeout(OPERATION_TIMEOUT);
        config
    }
}

#[derive(Debug, Default)]
pub struct Counters {
    operations: AtomicU64,
    errors: AtomicU64,
    last_error: std::sync::Mutex<Option<String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub operations: u64,
    pub errors: u64,
}

impl Counters {
    pub fn snapshot(&self) -> Counts {
        Counts {
            operations: self.operations.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok()?.clone()
    }

    fn record(&self, result: Result<()>) {
        match result {
            Ok(()) => {
                self.operations.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => {
                self.errors.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut last) = self.last_error.lock() {
                    *last = Some(format!("{error:#}"));
                }
            }
        }
    }
}

/// Waits until Proxy accepts and authenticates a client and answers a query.
///
/// A published port accepts TCP connections before Proxy listens behind it, so
/// a successful connect alone does not show that Proxy is ready.
pub async fn wait_until_ready(target: &Target, deadline: Duration) -> Result<()> {
    let started = tokio::time::Instant::now();
    loop {
        let attempt = async {
            let client = connect(target).await?;
            client.simple_query("SELECT 1").await?;
            anyhow::Ok(())
        };
        match timeout(OPERATION_TIMEOUT, attempt).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) if started.elapsed() >= deadline => {
                return Err(error.context("Proxy did not accept a client connection"));
            }
            Err(_) if started.elapsed() >= deadline => {
                anyhow::bail!("Proxy did not accept a client connection before the deadline");
            }
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// Runs the workload until the returned future is dropped.
pub async fn run(config: Config, target: Target, counters: Arc<Counters>) {
    let mut loops = JoinSet::new();
    for connection in 0..config.connections {
        let (target, counters) = (target.clone(), Arc::clone(&counters));
        let interval = config.interval;
        match config.workload {
            Workload::PooledQueries => {
                loops.spawn(pooled_queries(connection, interval, target, counters));
            }
            Workload::IdlePool => {
                loops.spawn(idle_pool(interval, target, counters));
            }
            workload => {
                loops.spawn(churn(workload, interval, target, counters));
            }
        }
    }
    while loops.join_next().await.is_some() {}
}

async fn churn(workload: Workload, interval: Duration, target: Target, counters: Arc<Counters>) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let connect = async {
            match workload {
                Workload::TcpProbe => tcp_probe(&target).await,
                Workload::StartupProbe => startup_probe(&target).await,
                Workload::LoginChurn | Workload::IdlePool | Workload::PooledQueries => {
                    login(&target).await
                }
            }
        };
        counters.record(
            timeout(OPERATION_TIMEOUT, connect)
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("connect timed out"))),
        );
    }
}

async fn tcp_probe(target: &Target) -> Result<()> {
    let mut stream = TcpStream::connect(target.address).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn startup_probe(target: &Target) -> Result<()> {
    let mut stream = TcpStream::connect(target.address).await?;
    stream
        .write_all(&startup_message(&target.user, &target.database))
        .await?;
    // Proxy answers a startup message with an authentication request.
    let tag = stream.read_u8().await?;
    let length = stream.read_i32().await?;
    anyhow::ensure!(
        tag == b'R' && (8..=1024).contains(&length),
        "expected an authentication request, got message {:?} of length {length}",
        char::from(tag)
    );
    let mut body = vec![0; usize::try_from(length - 4)?];
    stream.read_exact(&mut body).await?;
    stream.shutdown().await?;
    Ok(())
}

fn startup_message(user: &str, database: &str) -> Vec<u8> {
    let mut body = PROTOCOL_VERSION.to_be_bytes().to_vec();
    for field in ["user", user, "database", database] {
        body.extend_from_slice(field.as_bytes());
        body.push(0);
    }
    body.push(0);
    let length = i32::try_from(body.len() + 4).unwrap_or(i32::MAX);
    let mut message = length.to_be_bytes().to_vec();
    message.extend_from_slice(&body);
    message
}

async fn login(target: &Target) -> Result<()> {
    let (client, connection) = target.postgres_config().connect(NoTls).await?;
    let connection = tokio::spawn(connection);
    // Dropping the client makes the connection task send `Terminate`.
    drop(client);
    connection.await??;
    Ok(())
}

/// One idle pooled connection. It counts a connect each time it opens, then
/// holds the connection without sending anything until Proxy closes it. The
/// interval only bounds how fast it retries when connecting fails.
async fn idle_pool(interval: Duration, target: Target, counters: Arc<Counters>) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let opened = async {
            timeout(OPERATION_TIMEOUT, target.postgres_config().connect(NoTls))
                .await
                .context("connect timed out")?
                .context("connecting to Proxy")
        };
        match opened.await {
            Ok((client, connection)) => {
                counters.record(Ok(()));
                // Ends when Proxy closes the connection. The error it ends
                // with is the expected outcome, not a failed operation.
                let _ = connection.await;
                drop(client);
            }
            Err(error) => counters.record(Err(error)),
        }
    }
}

async fn connect(target: &Target) -> Result<Client> {
    let (client, connection) = target
        .postgres_config()
        .connect(NoTls)
        .await
        .context("connecting to Proxy")?;
    tokio::spawn(connection);
    Ok(client)
}

/// One pooled connection. It runs a five-statement cycle over a row it owns,
/// so the table stays the same size however long the run lasts.
///
/// Statements are sent as text, which `tokio-postgres` prepares, executes and
/// closes for every call. Each query therefore exercises Proxy's
/// per-connection statement and portal bookkeeping as well as encryption.
async fn pooled_queries(
    connection: usize,
    interval: Duration,
    target: Target,
    counters: Arc<Counters>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut client: Option<Client> = None;
    let mut row = Row::new(connection);
    loop {
        ticker.tick().await;
        let step = async {
            if client.as_ref().is_none_or(Client::is_closed) {
                client = Some(connect(&target).await?);
                // A fresh connection cannot rely on a row left by the old one.
                row.restart();
            }
            let Some(client) = client.as_ref() else {
                anyhow::bail!("no client connection");
            };
            row.step(client).await
        };
        let result = timeout(OPERATION_TIMEOUT, step)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("query timed out")));
        if result.is_err() {
            client = None;
        }
        counters.record(result);
    }
}

/// The row a pooled connection is cycling through insert, two reads, update
/// and delete.
struct Row {
    connection: usize,
    cycle: u64,
    phase: u8,
    id: Option<i32>,
}

impl Row {
    fn new(connection: usize) -> Self {
        Self {
            connection,
            cycle: 0,
            phase: 0,
            id: None,
        }
    }

    fn restart(&mut self) {
        self.phase = 0;
        self.id = None;
    }

    fn email(&self) -> String {
        // The varying part leads, so two rows never share a prefix.
        format!("{}-{}-burn-in@example.com", self.cycle, self.connection)
    }

    fn salary(&self) -> i32 {
        30_000 + i32::try_from(self.cycle % 100_000).unwrap_or_default()
    }

    async fn step(&mut self, client: &Client) -> Result<()> {
        let (email, salary) = (self.email(), self.salary());
        match self.phase {
            0 => {
                let inserted = client
                    .query_one(
                        "INSERT INTO users (encrypted_email, encrypted_salary) VALUES ($1, $2) RETURNING id",
                        &[&email, &salary],
                    )
                    .await
                    .context("insert")?;
                self.id = Some(inserted.get(0));
            }
            1 => {
                let rows = client
                    .query(
                        "SELECT id, encrypted_email, encrypted_salary FROM users WHERE encrypted_email = $1",
                        &[&email],
                    )
                    .await
                    .context("select by email")?;
                // Proxy 2.2.5 can return other rows with similar emails, so the
                // check is that this connection's row is among them.
                let found = rows
                    .iter()
                    .any(|row| row.get::<_, String>(1) == email && row.get::<_, i32>(2) == salary);
                anyhow::ensure!(
                    found,
                    "select by email returned {} rows, none of which decrypts to ({email}, {salary})",
                    rows.len()
                );
            }
            2 => {
                client
                    .query(
                        "SELECT id, encrypted_salary FROM users WHERE encrypted_salary >= $1 LIMIT 10",
                        &[&salary],
                    )
                    .await
                    .context("select by salary range")?;
            }
            3 => {
                let id = self.id.context("update without an inserted row")?;
                let updated = client
                    .execute(
                        "UPDATE users SET encrypted_salary = $1 WHERE id = $2",
                        &[&(salary + 1), &id],
                    )
                    .await
                    .context("update")?;
                anyhow::ensure!(updated == 1, "update changed {updated} rows");
            }
            _ => {
                let id = self.id.take().context("delete without an inserted row")?;
                let deleted = client
                    .execute("DELETE FROM users WHERE id = $1", &[&id])
                    .await
                    .context("delete")?;
                anyhow::ensure!(deleted == 1, "delete removed {deleted} rows");
                self.cycle += 1;
            }
        }
        self.phase = (self.phase + 1) % 5;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_message_is_length_prefixed_protocol_three() {
        let message = startup_message("alice", "app");
        let length = i32::from_be_bytes(message[..4].try_into().unwrap());
        assert_eq!(usize::try_from(length).unwrap(), message.len());
        assert_eq!(message[4..8], 196_608_i32.to_be_bytes());
        assert_eq!(&message[8..], b"user\0alice\0database\0app\0\0");
    }

    #[test]
    fn parses_workload_names_and_units() {
        assert_eq!("tcp-probe".parse(), Ok(Workload::TcpProbe));
        assert_eq!("startup-probe".parse(), Ok(Workload::StartupProbe));
        assert_eq!("login-churn".parse(), Ok(Workload::LoginChurn));
        assert_eq!("idle-pool".parse(), Ok(Workload::IdlePool));
        assert_eq!("pooled-queries".parse(), Ok(Workload::PooledQueries));
        assert!("idle".parse::<Workload>().is_err());
        assert_eq!(Workload::TcpProbe.unit(), "connect");
        assert_eq!(Workload::PooledQueries.unit(), "query");
    }

    #[test]
    fn counters_separate_operations_from_errors() {
        let counters = Counters::default();
        counters.record(Ok(()));
        counters.record(Ok(()));
        counters.record(Err(anyhow::anyhow!("refused")));
        assert_eq!(
            counters.snapshot(),
            Counts {
                operations: 2,
                errors: 1,
            }
        );
        assert_eq!(counters.last_error().as_deref(), Some("refused"));
    }

    #[test]
    fn a_reconnected_pooled_connection_starts_its_cycle_again() {
        let mut row = Row::new(3);
        row.phase = 3;
        row.id = Some(7);
        row.restart();
        assert_eq!((row.phase, row.id), (0, None));
        assert_eq!(row.email(), "0-3-burn-in@example.com");
    }
}
