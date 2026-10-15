//! # Connection pooler compatibility matrix
//!
//! Runs Proxy in both pooler topologies, against each pooler, in both pool
//! modes, and checks the behaviours that per-connection state in Proxy puts at
//! risk:
//!
//! ```text
//! app -> pooler -> Proxy -> PostgreSQL     ("pooler in front of Proxy")
//! app -> Proxy -> pooler -> PostgreSQL     ("Proxy in front of pooler")
//! ```
//!
//! The services come from `tests/docker-compose.poolers.yml`. Run the matrix
//! with `mise run test:poolers`, which starts them, runs this module and tears
//! them down. The module is compiled only with the `pooler-tests` feature so
//! the regular integration suite never tries to reach the poolers.
//!
//! Every cell has an expected outcome in [`expectation`]. A cell either
//! [`Expect::Works`] or is known to [`Expect::Fail`] with a documented reason.
//! A cell that changes behaviour in either direction fails its test, so the
//! supported-topologies table in `docs/reference/connection-poolers.md` cannot
//! silently drift from what the code does.
//!
//! Each test prints one `MATRIX | ...` line with the observed outcome, so the
//! whole table can be read back from a run with
//! `grep '^MATRIX' <output>`.

mod checks;
mod matrix;

use std::fmt;
use std::future::Future;
use std::time::Duration;

/// Upper bound for one cell. A pool of one that is never released would
/// otherwise hang the run instead of failing the cell.
const CELL_TIMEOUT: Duration = Duration::from_secs(90);

/// Port of the Proxy (default keyset) that poolers in front of Proxy connect to.
const DIRECT_PROXY_PORT: u16 = 6432;
/// Port of the Proxy without a default keyset.
const DIRECT_MULTITENANT_PROXY_PORT: u16 = 6442;

/// A pooler under test, and the Proxy instance that sits in front of it.
#[derive(Clone, Copy, Debug)]
pub struct Pooler {
    /// Name used in the results table.
    pub name: &'static str,
    /// Host port of the pooler.
    pub port: u16,
    /// Host port of the Proxy whose upstream is this pooler.
    pub proxy_in_front_port: u16,
}

pub const PGBOUNCER_1_19: Pooler = Pooler {
    name: "pgbouncer-1.19",
    port: 6461,
    proxy_in_front_port: 6451,
};

pub const PGBOUNCER_1_24: Pooler = Pooler {
    name: "pgbouncer-1.24 (max_prepared_statements=0)",
    port: 6462,
    proxy_in_front_port: 6452,
};

pub const PGBOUNCER_1_24_PS: Pooler = Pooler {
    name: "pgbouncer-1.24 (max_prepared_statements=200)",
    port: 6463,
    proxy_in_front_port: 6453,
};

pub const PGCAT_1_2: Pooler = Pooler {
    name: "pgcat-1.2 (prepared_statements_cache_size=0)",
    port: 6464,
    proxy_in_front_port: 6454,
};

pub const PGCAT_1_2_PS: Pooler = Pooler {
    name: "pgcat-1.2 (prepared_statements_cache_size=500)",
    port: 6465,
    proxy_in_front_port: 6455,
};

/// How the client reaches PostgreSQL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topology {
    /// app -> Proxy -> PostgreSQL. The control row: no pooler involved.
    Direct,
    /// app -> pooler -> Proxy -> PostgreSQL
    PoolerInFrontOfProxy,
    /// app -> Proxy -> pooler -> PostgreSQL
    ProxyInFrontOfPooler,
}

impl fmt::Display for Topology {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Topology::Direct => "app -> Proxy -> PG",
            Topology::PoolerInFrontOfProxy => "app -> pooler -> Proxy -> PG",
            Topology::ProxyInFrontOfPooler => "app -> Proxy -> pooler -> PG",
        })
    }
}

/// Pooler pool mode. [`Topology::Direct`] cells use [`PoolMode::Session`]:
/// a direct connection is a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolMode {
    Session,
    Transaction,
}

impl PoolMode {
    fn alias_suffix(self) -> &'static str {
        match self {
            PoolMode::Session => "session",
            PoolMode::Transaction => "txn",
        }
    }
}

impl fmt::Display for PoolMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PoolMode::Session => "session",
            PoolMode::Transaction => "transaction",
        })
    }
}

/// What a cell exercises. See the functions in [`checks`] for details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// The pooler accepts the protocol 3.2 startup packet Proxy sends upstream.
    StartupProtocol,
    /// Simple protocol INSERT, SELECT (by id and by encrypted equality), UPDATE, DELETE.
    CrudSimple,
    /// Extended protocol with the unnamed statement: Parse/Bind/Execute/Sync in one round trip.
    CrudExtendedUnnamed,
    /// Extended protocol with named statements, prepared once and executed in later transactions.
    CrudExtendedNamed,
    /// A named statement prepared on one server connection and executed on another.
    PreparedStatementSplit,
    /// A session keyset set inside an explicit transaction must not cross clients.
    KeysetIsolation,
    /// PgCat cleans a standalone session keyset SET when returning a connection.
    StandaloneKeysetCleanup,
    /// PgCat cleans a standalone mapping SET when returning a connection.
    StandaloneMappingCleanup,
    /// `SET LOCAL CIPHERSTASH.KEYSET_ID` inside a transaction must not outlive it.
    SetLocalKeysetIsolation,
    /// Mapping disabled inside an explicit transaction must not cross clients.
    DisableMappingIsolation,
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Check::StartupProtocol => "upstream pooler protocol 3.2 startup (CRUD not exercised)",
            Check::CrudSimple => "simple protocol CRUD",
            Check::CrudExtendedUnnamed => "extended protocol, unnamed statements",
            Check::CrudExtendedNamed => "extended protocol, named statements",
            Check::PreparedStatementSplit => "named statement executed on another connection",
            Check::KeysetIsolation => "session SET KEYSET_ID inside BEGIN/COMMIT isolation",
            Check::StandaloneKeysetCleanup => "standalone SET KEYSET_ID cleanup",
            Check::StandaloneMappingCleanup => "standalone SET UNSAFE_DISABLE_MAPPING cleanup",
            Check::SetLocalKeysetIsolation => "SET LOCAL CIPHERSTASH.KEYSET_ID isolation",
            Check::DisableMappingIsolation => {
                "session SET UNSAFE_DISABLE_MAPPING inside BEGIN/COMMIT isolation"
            }
        })
    }
}

/// One cell of the matrix.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub pooler: Option<Pooler>,
    pub topology: Topology,
    pub mode: PoolMode,
    pub check: Check,
}

/// The documented outcome of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// The check passes.
    Works,
    /// The check fails, for the stated reason. The cell is unsupported.
    Fail(Incompatibility, &'static str),
}

/// A specific, observed reason a topology is unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Incompatibility {
    Protocol32Refused,
    PreparedStatementMissing,
    KeysetCrossClient,
    MappingCrossClient,
}

/// Failed probes never count as evidence of an incompatibility.
#[derive(Debug)]
pub enum ProbeFailure {
    Incompatible(Incompatibility, String),
    Infrastructure(String),
    Unexpected(String),
    Inconclusive(String),
    Timeout(Duration),
}

impl From<String> for ProbeFailure {
    fn from(reason: String) -> Self {
        Self::Unexpected(reason)
    }
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incompatible(kind, evidence) => write!(f, "incompatible ({kind:?}): {evidence}"),
            Self::Infrastructure(reason) => write!(f, "infrastructure failure: {reason}"),
            Self::Unexpected(reason) => write!(f, "unexpected failure: {reason}"),
            Self::Inconclusive(reason) => write!(f, "inconclusive: {reason}"),
            Self::Timeout(duration) => write!(f, "timeout after {duration:?}"),
        }
    }
}

/// A database a test client connects to: a host port and the database name
/// that selects the pool (or, for Proxy directly, the real database).
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub port: u16,
    pub dbname: String,
}

/// Which Proxy a pooler in front of Proxy forwards to, and with what pool size.
#[derive(Clone, Copy, Debug)]
enum ProxyPool {
    /// Proxy with a default keyset, pool of two.
    Shared,
    /// Proxy with a default keyset, pool of one.
    Single,
    /// Proxy without a default keyset, pool of one.
    Multitenant,
    /// Proxy without a default keyset, pool of one, reserved for `SET LOCAL`.
    MultitenantLocal,
}

impl Cell {
    fn endpoint(&self, pool: ProxyPool) -> Endpoint {
        match (self.topology, self.pooler) {
            (Topology::Direct, _) => Endpoint {
                port: match pool {
                    ProxyPool::Shared | ProxyPool::Single => DIRECT_PROXY_PORT,
                    ProxyPool::Multitenant | ProxyPool::MultitenantLocal => {
                        DIRECT_MULTITENANT_PROXY_PORT
                    }
                },
                dbname: "cipherstash".to_string(),
            },
            (Topology::PoolerInFrontOfProxy, Some(pooler)) => {
                let prefix = match pool {
                    ProxyPool::Shared => "proxy",
                    ProxyPool::Single => "proxy_one",
                    ProxyPool::Multitenant => "proxy_mt",
                    ProxyPool::MultitenantLocal => "proxy_mt_local",
                };
                Endpoint {
                    port: pooler.port,
                    dbname: format!("{prefix}_{}", self.mode.alias_suffix()),
                }
            }
            (Topology::ProxyInFrontOfPooler, Some(pooler)) => Endpoint {
                port: pooler.proxy_in_front_port,
                dbname: format!("pg_{}", self.mode.alias_suffix()),
            },
            (topology, None) => panic!("{topology} needs a pooler"),
        }
    }
}

/// Runs one cell: executes its check, prints the observed outcome as a
/// `MATRIX` line, and asserts it matches [`expectation`].
pub async fn run_cell<F, Fut>(cell: Cell, timeout: Duration, check: F)
where
    F: FnOnce(Cell) -> Fut,
    Fut: Future<Output = Result<(), ProbeFailure>>,
{
    let observed = match tokio::time::timeout(timeout, check(cell)).await {
        Ok(result) => result,
        Err(_) => Err(ProbeFailure::Timeout(timeout)),
    };
    let expected = expectation(&cell);

    let pooler = cell.pooler.map_or("none", |p| p.name);
    let observed_text = match &observed {
        Ok(()) => "works".to_string(),
        Err(reason) => format!("FAILS: {}", reason.to_string().replace('\n', " ")),
    };
    println!(
        "MATRIX | {pooler} | {} | {} | {} | {observed_text} | expected: {expected:?}",
        cell.topology, cell.mode, cell.check,
    );

    match (expected, observed) {
        (Expect::Works, Ok(())) => {}
        (Expect::Fail(expected_kind, _), Err(ProbeFailure::Incompatible(actual_kind, _)))
            if expected_kind == actual_kind => {}
        (Expect::Fail(_, why), Ok(())) => panic!(
            "{cell:?} was expected to fail ({why}) but now works: update `expectation` and docs/reference/connection-poolers.md"
        ),
        (expected, Err(reason)) => {
            panic!("{cell:?} expected {expected:?}, observed {reason}")
        }
    }
}

/// Why a pooler cannot sit behind Proxy. The probe connects to the pooler
/// directly; Proxy's upstream connections are inferred to fail because Proxy
/// requests protocol 3.2 at startup.
const REJECTS_PROTOCOL_3_2: &str =
    "the pooler rejects a protocol 3.2 startup packet, which Proxy sends upstream, so Proxy's upstream connections through it fail (CRUD not run)";
/// Why a named statement fails when its Bind lands on another server connection.
const NO_PREPARED_STATEMENT_SUPPORT: &str =
    "the pooler does not track prepared statements, so Bind/Execute reach a server connection that never saw the Parse";
/// Why a session-level SET leaks in transaction mode.
const SESSION_SET_IN_TRANSACTION_MODE: &str =
    "a session-level SET changes the shared Proxy connection, which the pooler hands to other clients between transactions; use SET LOCAL";
/// Why a session-level SET leaks behind PgCat in session mode.
const PGCAT_UNTRACKED_TRANSACTION_SET: &str =
    "PgCat does not mark session SET inside explicit transactions for cleanup, so the next client inherits the Proxy connection's settings";

/// The supported-topologies table. Keep in sync with
/// `docs/reference/connection-poolers.md`.
pub fn expectation(cell: &Cell) -> Expect {
    let Some(pooler) = cell.pooler else {
        return Expect::Works;
    };
    let pgcat = pooler.name.starts_with("pgcat");
    let pooler_tracks_prepared_statements =
        pooler.port == PGBOUNCER_1_24_PS.port || pooler.port == PGCAT_1_2_PS.port;
    let accepts_protocol_3_2 =
        pooler.port == PGBOUNCER_1_24.port || pooler.port == PGBOUNCER_1_24_PS.port;

    match (cell.topology, cell.mode, cell.check) {
        (Topology::Direct, ..) => Expect::Works,

        (Topology::ProxyInFrontOfPooler, ..) if !accepts_protocol_3_2 => {
            Expect::Fail(Incompatibility::Protocol32Refused, REJECTS_PROTOCOL_3_2)
        }

        (_, PoolMode::Transaction, Check::PreparedStatementSplit)
            if !pooler_tracks_prepared_statements =>
        {
            Expect::Fail(
                Incompatibility::PreparedStatementMissing,
                NO_PREPARED_STATEMENT_SUPPORT,
            )
        }

        (
            Topology::PoolerInFrontOfProxy,
            PoolMode::Transaction,
            Check::KeysetIsolation | Check::DisableMappingIsolation,
        ) => Expect::Fail(
            match cell.check {
                Check::KeysetIsolation => Incompatibility::KeysetCrossClient,
                _ => Incompatibility::MappingCrossClient,
            },
            SESSION_SET_IN_TRANSACTION_MODE,
        ),

        (
            Topology::PoolerInFrontOfProxy,
            PoolMode::Session,
            Check::KeysetIsolation | Check::DisableMappingIsolation,
        ) if pgcat => Expect::Fail(
            match cell.check {
                Check::KeysetIsolation => Incompatibility::KeysetCrossClient,
                _ => Incompatibility::MappingCrossClient,
            },
            PGCAT_UNTRACKED_TRANSACTION_SET,
        ),

        _ => Expect::Works,
    }
}

#[cfg(test)]
mod runner_tests {
    use super::*;

    #[tokio::test]
    #[should_panic(expected = "timeout after")]
    async fn unsupported_cell_cannot_pass_when_probe_times_out() {
        run_cell(unsupported_cell(), Duration::from_millis(1), |_| {
            std::future::pending()
        })
        .await;
    }

    /// A scripted PostgreSQL peer is the external wire boundary. The runner
    /// still executes the real startup probe and evaluates its actual result.
    async fn startup_peer(response: &[u8]) -> (Endpoint, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let endpoint = Endpoint {
            port: listener.local_addr().unwrap().port(),
            dbname: "pg_session".into(),
        };
        let response = response.to_vec();
        let task = tokio::spawn(async move {
            for (version, reply) in [
                (196608u32, b"R\0\0\0\x08\0\0\0\x03".to_vec()),
                (196610, response),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let len = socket.read_u32().await.unwrap();
                let mut packet = vec![0; (len - 4) as usize];
                socket.read_exact(&mut packet).await.unwrap();
                assert_eq!(&packet[..4], &version.to_be_bytes());
                socket.write_all(&reply).await.unwrap();
            }
        });
        (endpoint, task)
    }

    fn startup_cell() -> Cell {
        Cell {
            topology: Topology::ProxyInFrontOfPooler,
            check: Check::StartupProtocol,
            ..unsupported_cell()
        }
    }

    #[tokio::test]
    async fn startup_rejection_requires_protocol_evidence() {
        let body = b"SFATAL\0C08P01\0Mbad packet header: '0000002e00030002'\0\0";
        let mut reply = vec![b'E'];
        reply.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        reply.extend_from_slice(body);
        let (endpoint, peer) = startup_peer(&reply).await;
        run_cell(startup_cell(), CELL_TIMEOUT, |_| async {
            checks::startup_at(&endpoint, PGBOUNCER_1_19).await
        })
        .await;
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn startup_rejection_of_another_packet_does_not_prove_protocol_rejection() {
        for header in ["0000002e00030000", "0000002a00030002"] {
            let body = format!("SFATAL\0C08P01\0Mbad packet header: '{header}'\0\0");
            let mut reply = vec![b'E'];
            reply.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
            reply.extend_from_slice(body.as_bytes());
            let (endpoint, peer) = startup_peer(&reply).await;
            let result = checks::startup_at(&endpoint, PGBOUNCER_1_19).await;
            peer.await.unwrap();
            assert!(matches!(result, Err(ProbeFailure::Unexpected(_))));
        }
    }

    #[tokio::test]
    async fn startup_closed_connection_does_not_prove_protocol_rejection() {
        let (endpoint, peer) = startup_peer(b"").await;
        let result = tokio::spawn(run_cell(startup_cell(), CELL_TIMEOUT, |_| async move {
            checks::startup_at(&endpoint, PGBOUNCER_1_19).await
        }))
        .await;
        peer.await.unwrap();
        assert!(result.unwrap_err().is_panic());
    }

    #[tokio::test]
    async fn startup_authentication_failure_does_not_prove_protocol_rejection() {
        let body = b"SFATAL\0C28P01\0Mpassword authentication failed\0\0";
        let mut reply = vec![b'E'];
        reply.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        reply.extend_from_slice(body);
        let (endpoint, peer) = startup_peer(&reply).await;
        let result = tokio::spawn(run_cell(startup_cell(), CELL_TIMEOUT, |_| async move {
            checks::startup_at(&endpoint, PGBOUNCER_1_19).await
        }))
        .await;
        peer.await.unwrap();
        assert!(result.unwrap_err().is_panic());
    }

    async fn pgcat_peer(replies: Vec<Vec<u8>>) -> (Endpoint, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let endpoint = Endpoint {
            port: listener.local_addr().unwrap().port(),
            dbname: "pg_session".into(),
        };
        let task = tokio::spawn(async move {
            for (index, reply) in replies.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let len = socket.read_u32().await.unwrap();
                let mut packet = vec![0; (len - 4) as usize];
                socket.read_exact(&mut packet).await.unwrap();
                let version: u32 = if index % 3 == 1 { 196610 } else { 196608 };
                assert_eq!(&packet[..4], &version.to_be_bytes());
                if reply == b"STALL" {
                    std::future::pending::<()>().await;
                }
                socket.write_all(&reply).await.unwrap();
            }
        });
        (endpoint, task)
    }

    #[tokio::test]
    async fn pinned_pgcat_requires_repeated_version_specific_refusal_and_authenticated_controls() {
        // AuthenticationOk followed by ReadyForQuery(Idle), not merely an
        // authentication challenge or an open socket.
        let healthy = b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I".to_vec();
        let replies = vec![
            healthy.clone(),
            vec![],
            healthy.clone(),
            healthy.clone(),
            vec![],
            healthy.clone(),
        ];
        let (endpoint, peer) = pgcat_peer(replies).await;
        run_cell(
            Cell {
                pooler: Some(PGCAT_1_2),
                ..startup_cell()
            },
            CELL_TIMEOUT,
            |_| async { checks::startup_at(&endpoint, PGCAT_1_2).await },
        )
        .await;
        peer.await.unwrap();

        for replies in [
            vec![vec![]],                                        // early EOF on the first control
            vec![healthy.clone(), vec![]], // service crashes before post-control
            vec![healthy.clone(), b"E\0\0\0\x03".to_vec()], // malformed 3.2 response
            vec![healthy.clone(), b"E\0\0\0\x08bad\0".to_vec()], // unrelated 3.2 error
        ] {
            let (endpoint, peer) = pgcat_peer(replies).await;
            let result = tokio::spawn(run_cell(
                Cell {
                    pooler: Some(PGCAT_1_2),
                    ..startup_cell()
                },
                Duration::from_secs(2),
                |_| async move { checks::startup_at(&endpoint, PGCAT_1_2).await },
            ))
            .await;
            peer.await.unwrap();
            assert!(result.unwrap_err().is_panic());
        }

        let healthy = b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I".to_vec();
        let (endpoint, peer) = pgcat_peer(vec![healthy, b"STALL".to_vec()]).await;
        let result = tokio::spawn(run_cell(
            Cell {
                pooler: Some(PGCAT_1_2),
                ..startup_cell()
            },
            Duration::from_millis(100),
            |_| async move { checks::startup_at(&endpoint, PGCAT_1_2).await },
        ))
        .await;
        peer.abort();
        assert!(result.unwrap_err().is_panic());
    }

    fn unsupported_cell() -> Cell {
        Cell {
            pooler: Some(PGBOUNCER_1_19),
            topology: Topology::PoolerInFrontOfProxy,
            mode: PoolMode::Transaction,
            check: Check::KeysetIsolation,
        }
    }

    #[tokio::test]
    async fn unsupported_cell_requires_its_specific_incompatibility() {
        let outcomes = [
            ProbeFailure::Infrastructure("connection refused".into()),
            ProbeFailure::Unexpected("permission denied".into()),
            ProbeFailure::Inconclusive("client B returned no row".into()),
            ProbeFailure::Timeout(Duration::from_secs(90)),
            ProbeFailure::Incompatible(
                Incompatibility::MappingCrossClient,
                "wrong setting crossed clients".into(),
            ),
        ];
        for observed in outcomes {
            let result = tokio::spawn(run_cell(unsupported_cell(), CELL_TIMEOUT, |_| async {
                Err(observed)
            }))
            .await;
            assert!(result.unwrap_err().is_panic());
        }
        run_cell(unsupported_cell(), CELL_TIMEOUT, |_| async {
            Err(ProbeFailure::Incompatible(
                Incompatibility::KeysetCrossClient,
                "client B decrypted client A's row".into(),
            ))
        })
        .await;
    }

    #[tokio::test]
    #[should_panic(expected = "now works")]
    async fn unexpected_compatibility_requires_review() {
        run_cell(unsupported_cell(), CELL_TIMEOUT, |_| async { Ok(()) }).await;
    }

    #[tokio::test]
    async fn supported_cell_requires_successful_probe() {
        let cell = Cell {
            pooler: None,
            topology: Topology::Direct,
            ..unsupported_cell()
        };
        let result = tokio::spawn(run_cell(cell, CELL_TIMEOUT, |_| async {
            Err(ProbeFailure::Incompatible(
                Incompatibility::KeysetCrossClient,
                "client B decrypted client A's row".into(),
            ))
        }))
        .await;
        assert!(result.unwrap_err().is_panic());
        run_cell(cell, CELL_TIMEOUT, |_| async { Ok(()) }).await;
    }

    #[tokio::test]
    #[should_panic(expected = "observed")]
    async fn unsupported_cell_rejects_setup_failure() {
        run_cell(
            Cell {
                pooler: Some(PGBOUNCER_1_19),
                topology: Topology::PoolerInFrontOfProxy,
                mode: PoolMode::Transaction,
                check: Check::KeysetIsolation,
            },
            CELL_TIMEOUT,
            |_| async { Err("setup: connection refused".to_string().into()) },
        )
        .await;
    }
}
