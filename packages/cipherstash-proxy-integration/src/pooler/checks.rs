//! The checks run in each cell of the pooler matrix.
//!
//! Checks report typed incompatibilities only when the probe produces evidence.
//! Setup failures, unrelated errors and inconclusive observations remain failures.

use super::{Cell, Check, Endpoint, Incompatibility, PoolMode, Pooler, ProbeFailure, ProxyPool};
use crate::common::{random_id, random_string};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_postgres::{types::Type, Client, NoTls, SimpleQueryMessage};

/// A client connection plus the task driving it, so a check can close a
/// connection and know the pooler has seen the disconnect.
struct Connection {
    client: Client,
    task: JoinHandle<()>,
}

impl Connection {
    async fn open(endpoint: &Endpoint) -> Result<Self, ProbeFailure> {
        let mut config = tokio_postgres::Config::new();
        config
            .host("localhost")
            .port(endpoint.port)
            .user("cipherstash")
            .password("p@ssword")
            .dbname(&endpoint.dbname)
            .connect_timeout(Duration::from_secs(10));

        let (client, connection) = config.connect(NoTls).await.map_err(|e| {
            ProbeFailure::Infrastructure(format!(
                "could not connect to {endpoint:?}: {}",
                describe(&e)
            ))
        })?;

        let task = tokio::spawn(async move {
            // Connection errors surface through the client; nothing to do here.
            let _ = connection.await;
        });

        Ok(Self { client, task })
    }

    /// Sends Terminate and waits for the socket to close.
    async fn close(self) {
        drop(self.client);
        let _ = self.task.await;
    }

    async fn simple(&self, sql: &str) -> Result<Vec<SimpleQueryMessage>, String> {
        self.client
            .simple_query(sql)
            .await
            .map_err(|e| describe(&e))
    }

    /// Runs a simple query and returns the first column of the first row.
    async fn simple_value(&self, sql: &str) -> Result<Option<String>, String> {
        Ok(first_value(&self.simple(sql).await?))
    }

    async fn backend_pid(&self) -> Result<String, String> {
        self.simple_value("SELECT pg_backend_pid()")
            .await?
            .ok_or_else(|| "pg_backend_pid() returned no row".to_string())
    }
}

/// Renders a client error with the server's message when there is one.
/// `tokio_postgres::Error`'s Display hides the server text behind "db error".
fn describe(error: &tokio_postgres::Error) -> String {
    match error.as_db_error() {
        Some(db) => format!("{}: {}", db.severity(), db.message()),
        None => match std::error::Error::source(error) {
            Some(source) => format!("{error}: {source}"),
            None => error.to_string(),
        },
    }
}

fn first_value(messages: &[SimpleQueryMessage]) -> Option<String> {
    messages.iter().find_map(|message| match message {
        SimpleQueryMessage::Row(row) => row.get(0).map(str::to_string),
        _ => None,
    })
}

fn expect_value(actual: Option<String>, expected: &str, step: &str) -> Result<(), String> {
    match actual {
        Some(value) if value == expected => Ok(()),
        other => Err(format!(
            "{step}: expected {expected:?}, got {other:?} (ciphertext here means the result was not decrypted)"
        )),
    }
}

/// The keyset multitenant checks switch to. Any keyset the configured client
/// can use will do; `mise run test:poolers` passes the workspace default.
fn tenant_keyset_id() -> Result<String, ProbeFailure> {
    std::env::var("CS_TEST_POOLER_KEYSET_ID").map_err(|_| ProbeFailure::Infrastructure(
        "CS_TEST_POOLER_KEYSET_ID must name an accessible keyset (run via `mise run test:poolers`)".to_string(),
    ))
}

/// True when an error is Proxy refusing to encrypt or decrypt because the
/// connection has no keyset: the fail-closed outcome the isolation checks want.
fn is_missing_keyset(error: &str) -> bool {
    matches!(
        error,
        "ERROR: Missing keyset identifer." | "FATAL: Missing keyset identifer."
    )
}

pub async fn crud_simple(cell: Cell) -> Result<(), ProbeFailure> {
    let endpoint = cell.endpoint(ProxyPool::Shared);
    let conn = Connection::open(&endpoint).await?;
    let id = random_id();
    let first = random_string();
    let second = random_string();

    conn.simple(&format!(
        "INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{first}')"
    ))
    .await
    .map_err(|e| format!("insert: {e}"))?;

    let by_id = conn
        .simple_value(&format!(
            "SELECT encrypted_text FROM encrypted WHERE id = {id}"
        ))
        .await
        .map_err(|e| format!("select by id: {e}"))?;
    expect_value(by_id, &first, "select by id")?;

    let by_value = conn
        .simple_value(&format!(
            "SELECT encrypted_text FROM encrypted WHERE encrypted_text = '{first}'"
        ))
        .await
        .map_err(|e| format!("select by encrypted equality: {e}"))?;
    expect_value(by_value, &first, "select by encrypted equality")?;

    conn.simple(&format!(
        "UPDATE encrypted SET encrypted_text = '{second}' WHERE id = {id}"
    ))
    .await
    .map_err(|e| format!("update: {e}"))?;
    let updated = conn
        .simple_value(&format!(
            "SELECT encrypted_text FROM encrypted WHERE id = {id}"
        ))
        .await
        .map_err(|e| format!("select after update: {e}"))?;
    expect_value(updated, &second, "select after update")?;

    conn.simple(&format!("DELETE FROM encrypted WHERE id = {id}"))
        .await
        .map_err(|e| format!("delete: {e}"))?;
    let remaining = conn
        .simple_value(&format!("SELECT count(*) FROM encrypted WHERE id = {id}"))
        .await
        .map_err(|e| format!("count after delete: {e}"))?;
    expect_value(remaining, "0", "count after delete")?;

    conn.close().await;
    Ok(())
}

pub async fn crud_extended_unnamed(cell: Cell) -> Result<(), ProbeFailure> {
    let endpoint = cell.endpoint(ProxyPool::Shared);
    let conn = Connection::open(&endpoint).await?;
    let client = &conn.client;
    let id = random_id();
    let value = random_string();

    client
        .query_typed(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
            &[(&id, Type::INT8), (&value, Type::TEXT)],
        )
        .await
        .map_err(|e| format!("insert: {}", describe(&e)))?;

    let rows = client
        .query_typed(
            "SELECT encrypted_text FROM encrypted WHERE id = $1",
            &[(&id, Type::INT8)],
        )
        .await
        .map_err(|e| format!("select by id: {}", describe(&e)))?;
    let actual = rows
        .first()
        .map(|row| row.try_get::<_, String>(0))
        .transpose()
        .map_err(|e| format!("select by id: could not read text: {}", describe(&e)))?;
    expect_value(actual, &value, "select by id")?;

    let rows = client
        .query_typed(
            "SELECT id FROM encrypted WHERE encrypted_text = $1",
            &[(&value, Type::TEXT)],
        )
        .await
        .map_err(|e| format!("select by encrypted equality: {}", describe(&e)))?;
    let found: Option<i64> = rows.first().map(|row| row.get(0));
    if found != Some(id) {
        return Err(
            format!("select by encrypted equality: expected id {id}, got {found:?}").into(),
        );
    }

    conn.close().await;
    Ok(())
}

pub async fn crud_extended_named(cell: Cell) -> Result<(), ProbeFailure> {
    let endpoint = cell.endpoint(ProxyPool::Shared);
    let conn = Connection::open(&endpoint).await?;
    let client = &conn.client;
    let id = random_id();
    let value = random_string();

    // Keep Parse and every Bind on the same backend. The separate split probe
    // deterministically tests reuse across transaction boundaries.
    conn.simple("BEGIN")
        .await
        .map_err(|e| format!("begin: {e}"))?;

    let insert = client
        .prepare("INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)")
        .await
        .map_err(|e| format!("prepare insert: {}", describe(&e)))?;
    client
        .execute(&insert, &[&id, &value])
        .await
        .map_err(|e| format!("execute insert: {}", describe(&e)))?;

    let select = client
        .prepare("SELECT encrypted_text FROM encrypted WHERE id = $1")
        .await
        .map_err(|e| format!("prepare select: {}", describe(&e)))?;

    for attempt in 1..=3 {
        let rows = client
            .query(&select, &[&id])
            .await
            .map_err(|e| format!("execute select #{attempt}: {}", describe(&e)))?;
        let actual = rows
            .first()
            .map(|row| row.try_get::<_, String>(0))
            .transpose()
            .map_err(|e| {
                format!(
                    "execute select #{attempt}: could not read text: {}",
                    describe(&e)
                )
            })?;
        expect_value(actual, &value, &format!("execute select #{attempt}"))?;
    }

    conn.simple("COMMIT")
        .await
        .map_err(|e| format!("commit: {e}"))?;
    conn.close().await;
    Ok(())
}

/// Forces a named statement's Bind/Execute onto a different server connection
/// from its Parse, which is what a transaction-mode pooler does under load.
///
/// The pool behind the endpoint has two server connections. Client A prepares
/// the statement inside a transaction (so the Parse and `pg_backend_pid()` land
/// on the same server connection), then other clients open transactions until
/// one of them holds that server connection. Client A then executes the
/// statement, which can only run on the other server connection.
pub async fn prepared_statement_split(cell: Cell) -> Result<(), ProbeFailure> {
    assert_eq!(
        cell.mode,
        PoolMode::Transaction,
        "a session-mode client cannot change server connection"
    );
    const POOL_SIZE: usize = 2;

    let endpoint = cell.endpoint(ProxyPool::Shared);
    let id = random_id();
    let value = random_string();

    let a = Connection::open(&endpoint).await?;
    a.simple(&format!(
        "INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{value}')"
    ))
    .await
    .map_err(|e| format!("seed insert: {e}"))?;

    a.simple("BEGIN").await.map_err(|e| format!("begin: {e}"))?;
    let sql = "SELECT encrypted_text FROM encrypted WHERE id = $1";
    let statement = match a.client.prepare(sql).await {
        Ok(statement) => statement,
        // Statement names restart at s0 in every test process, and without
        // prepared statement support a transaction-mode pooler leaves earlier
        // clients' statements on its server connections. That collision is a
        // failure of its own; clear it so this check measures the split.
        // Only on collision: a pooler that manages statements itself (PgCat's
        // cache) is confused by a DEALLOCATE it did not issue.
        // Only where the cell expects missing prepared statement support: a
        // collision behind a pooler that should track statements is a
        // regression, so it falls through and fails the cell.
        Err(e)
            if e.code() == Some(&tokio_postgres::error::SqlState::DUPLICATE_PSTATEMENT)
                && matches!(
                    super::expectation(&cell),
                    super::Expect::Fail(Incompatibility::PreparedStatementMissing, _)
                ) =>
        {
            a.simple("ROLLBACK")
                .await
                .map_err(|e| format!("rollback: {e}"))?;
            a.simple("BEGIN").await.map_err(|e| format!("begin: {e}"))?;
            a.simple("DEALLOCATE ALL")
                .await
                .map_err(|e| format!("deallocate: {e}"))?;
            a.client
                .prepare(sql)
                .await
                .map_err(|e| format!("prepare after deallocate: {}", describe(&e)))?
        }
        Err(e) => return Err(format!("prepare: {}", describe(&e)).into()),
    };
    let prepared_on = a.backend_pid().await?;
    a.simple("COMMIT")
        .await
        .map_err(|e| format!("commit: {e}"))?;

    // Occupy the server connection that holds the prepared statement.
    let mut holders = Vec::new();
    let mut occupied = false;
    for _ in 0..POOL_SIZE {
        let holder = Connection::open(&endpoint).await?;
        holder
            .simple("BEGIN")
            .await
            .map_err(|e| format!("holder begin: {e}"))?;
        let pid = holder.backend_pid().await?;
        let holds_statement = pid == prepared_on;
        holders.push((holder, holds_statement));
        if holds_statement {
            occupied = true;
            break;
        }
    }
    if !occupied {
        return Err(format!(
            "setup: could not occupy server connection {prepared_on}; pool is larger than {POOL_SIZE}?"
        ).into());
    }

    // Release every other server connection so client A has somewhere to go.
    let mut statement_holder = None;
    for (holder, holds_statement) in holders {
        if holds_statement {
            statement_holder = Some(holder);
        } else {
            holder
                .simple("COMMIT")
                .await
                .map_err(|e| format!("holder commit: {e}"))?;
            holder.close().await;
        }
    }

    let result = a.client.query(&statement, &[&id]).await;

    if let Some(holder) = statement_holder {
        holder
            .simple("COMMIT")
            .await
            .map_err(|e| format!("release statement holder: {e}"))?;
        holder.close().await;
    }

    let rows = result.map_err(|e| {
        let evidence = format!(
            "execute on another server connection after occupying backend {prepared_on}: {}",
            describe(&e)
        );
        if e.code() == Some(&tokio_postgres::error::SqlState::INVALID_SQL_STATEMENT_NAME) {
            ProbeFailure::Incompatible(Incompatibility::PreparedStatementMissing, evidence)
        } else {
            ProbeFailure::Unexpected(evidence)
        }
    })?;
    let actual = rows
        .first()
        .map(|row| row.try_get::<_, String>(0))
        .transpose()
        .map_err(|e| format!("could not read text: {}", describe(&e)))?;
    expect_value(actual, &value, "execute on another server connection")?;

    a.close().await;
    Ok(())
}

/// Fails if a client that has *not* set a keyset can encrypt through the pool:
/// the pooled Proxy connection already carries some other client's keyset.
///
/// The probe compares an encrypted column with a literal, which Proxy must
/// encrypt whether or not any row matches, so it needs a keyset even on an
/// empty result.
async fn assert_pool_has_no_keyset(endpoint: &Endpoint) -> Result<(), ProbeFailure> {
    let probe = Connection::open(endpoint).await?;
    let result = probe
        .simple_value(&format!(
            "SELECT id FROM encrypted WHERE encrypted_text = '{}'",
            random_string()
        ))
        .await;
    probe.close().await;
    match result {
        Err(e) if is_missing_keyset(&e) => Ok(()),
        Err(e) => Err(format!("precondition probe failed unexpectedly: {e}").into()),
        Ok(_) => Err(ProbeFailure::Inconclusive(
            "precondition: a fresh client already carries a keyset before client A sets one; reset the pool before probing isolation".to_string(),
        )),
    }
}

/// What client B, which never set a keyset, can do on the shared pool.
async fn observe_client_without_keyset(
    b: &Connection,
    id: i64,
    secret: &str,
) -> Result<(), ProbeFailure> {
    // Prove B has a working server connection, so a later error is Proxy's
    // decision rather than a broken connection.
    b.simple("SELECT 1")
        .await
        .map_err(|e| format!("client B could not run SELECT 1: {e}"))?;

    match b
        .simple_value(&format!(
            "SELECT encrypted_text FROM encrypted WHERE id = {id}"
        ))
        .await
    {
        Err(e) if is_missing_keyset(&e) => Ok(()),
        Err(e) => Err(ProbeFailure::Inconclusive(format!(
            "client B select failed unexpectedly: {e}"
        ))),
        Ok(Some(value)) if value == secret => Err(ProbeFailure::Incompatible(
            Incompatibility::KeysetCrossClient,
            "client B decrypted client A's row without setting a keyset".to_string(),
        )),
        Ok(other) => Err(ProbeFailure::Inconclusive(format!(
            "client B select returned {other:?}, expected a missing-keyset error or client A's plaintext"
        ))),
    }
}

async fn require_reused_backend(
    cell: Cell,
    prepared_on: &str,
    b: &Connection,
) -> Result<(), ProbeFailure> {
    if cell.topology == super::Topology::PoolerInFrontOfProxy {
        let observed_on = b.backend_pid().await?;
        if observed_on != prepared_on {
            return Err(ProbeFailure::Inconclusive(format!(
                "client B reached backend {observed_on}, but client A used {prepared_on}; isolation was not tested on a reused connection"
            )));
        }
    }
    Ok(())
}

/// Cleanup must run even when setup or the observation returns early. Roll back
/// unfinished transactions before restoring settings, and retain every failure.
async fn finish_isolation_probe(
    endpoint: &Endpoint,
    a: Option<Connection>,
    b: Option<Connection>,
    restore: &str,
    observed: Result<(), ProbeFailure>,
) -> Result<(), ProbeFailure> {
    let mut failures = Vec::new();
    for (name, connection) in [("client A", a), ("client B", b)] {
        if let Some(connection) = connection {
            if let Err(error) = connection.simple("ROLLBACK").await {
                failures.push(format!("restore {name} rollback: {error}"));
            }
            if let Err(error) = connection.simple(restore).await {
                failures.push(format!("restore {name} settings: {error}"));
            }
            connection.close().await;
        }
    }
    // Session-mode probes deliberately close A before observing B. Restore the
    // pool after all client disconnects too, including failed attempts to open B.
    match Connection::open(endpoint).await {
        Ok(connection) => {
            if let Err(error) = connection.simple(restore).await {
                failures.push(format!("restore probe pool: {error}"));
            }
            connection.close().await;
        }
        Err(error) => failures.push(format!("restore probe pool connection: {error}")),
    }
    if failures.is_empty() {
        observed
    } else {
        let cleanup = failures.join("; ");
        Err(ProbeFailure::Unexpected(match observed {
            Ok(()) => format!("probe cleanup failed: {cleanup}"),
            Err(error) => format!("{error}; probe cleanup failed: {cleanup}"),
        }))
    }
}

/// Client A sets a session keyset inside an explicit transaction; B never does.
/// PgCat does not mark this form for cleanup. Its separate standalone cleanup
/// cells seed under SET LOCAL first, then issue SET outside the transaction.
/// Behind a pool of one, B gets the server connection A used.
///
/// Transaction mode: A stays connected; B's statements run on A's server
/// connection between A's transactions.
/// Session mode: A disconnects first. PgBouncer always sends DISCARD ALL;
/// PgCat only sends RESET ALL for the standalone SET variant.
pub async fn keyset_isolation(cell: Cell) -> Result<(), ProbeFailure> {
    let endpoint = cell.endpoint(ProxyPool::Multitenant);
    let keyset = tenant_keyset_id()?;
    let id = random_id();
    let secret = random_string();

    assert_pool_has_no_keyset(&endpoint).await?;

    let mut client_a = Some(Connection::open(&endpoint).await?);
    let mut client_b = None;
    let observed = async {
        let a = client_a.as_ref().unwrap();
        let standalone = cell.check == Check::StandaloneKeysetCleanup;
        a.simple("BEGIN").await?;
        let scope = if standalone { "LOCAL " } else { "" };
        a.simple(&format!("SET {scope}CIPHERSTASH.KEYSET_ID = '{keyset}'"))
            .await
            .map_err(|e| format!("client A set keyset: {e}"))?;
        a.simple(&format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{secret}')"
        ))
        .await
        .map_err(|e| format!("setup: client A insert with its keyset: {e}"))?;
        let own = a
            .simple_value(&format!(
                "SELECT encrypted_text FROM encrypted WHERE id = {id}"
            ))
            .await
            .map_err(|e| format!("setup: client A select with its keyset: {e}"))?;
        expect_value(own, &secret, "setup: client A select with its keyset")?;

        let prepared_on = a.backend_pid().await?;
        a.simple("COMMIT").await?;
        if standalone {
            a.simple(&format!("SET CIPHERSTASH.KEYSET_ID = '{keyset}'"))
                .await?;
        }
        if cell.mode == PoolMode::Session {
            client_a.take().unwrap().close().await;
        }

        client_b = Some(Connection::open(&endpoint).await?);
        let b = client_b.as_ref().unwrap();
        require_reused_backend(cell, &prepared_on, b).await?;
        observe_client_without_keyset(b, id, &secret).await
    }
    .await;
    finish_isolation_probe(&endpoint, client_a, client_b, "RESET ALL", observed).await
}

/// Client A scopes a keyset to one explicit transaction with `SET LOCAL`;
/// after A commits, client B (no keyset) gets the same server connection.
pub async fn set_local_keyset_isolation(cell: Cell) -> Result<(), ProbeFailure> {
    let endpoint = cell.endpoint(ProxyPool::MultitenantLocal);
    let keyset = tenant_keyset_id()?;
    let id = random_id();
    let secret = random_string();

    assert_pool_has_no_keyset(&endpoint).await?;

    let a = Connection::open(&endpoint).await?;
    a.simple("BEGIN").await.map_err(|e| format!("begin: {e}"))?;
    a.simple(&format!("SET LOCAL CIPHERSTASH.KEYSET_ID = '{keyset}'"))
        .await
        .map_err(|e| format!("client A set local keyset: {e}"))?;
    a.simple(&format!(
        "INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{secret}')"
    ))
    .await
    .map_err(|e| format!("setup: client A insert inside its transaction: {e}"))?;
    let own = a
        .simple_value(&format!(
            "SELECT encrypted_text FROM encrypted WHERE id = {id}"
        ))
        .await
        .map_err(|e| format!("setup: client A select inside its transaction: {e}"))?;
    expect_value(
        own,
        &secret,
        "setup: client A select inside its transaction",
    )?;
    let prepared_on = a.backend_pid().await?;
    a.simple("COMMIT")
        .await
        .map_err(|e| format!("commit: {e}"))?;

    // On a direct connection the "other client" is A itself after COMMIT:
    // SET LOCAL must not outlive the transaction on the same connection either.
    let observed = match cell.topology {
        super::Topology::Direct => observe_client_without_keyset(&a, id, &secret).await,
        _ => {
            let b = Connection::open(&endpoint).await?;
            require_reused_backend(cell, &prepared_on, &b).await?;
            let observed = observe_client_without_keyset(&b, id, &secret).await;
            b.close().await;
            observed
        }
    };
    a.close().await;
    observed
}

/// Client A disables mapping; client B, which did not, must still get
/// plaintext. A leak shows up as B receiving the stored ciphertext.
pub async fn disable_mapping_isolation(cell: Cell) -> Result<(), ProbeFailure> {
    let endpoint = cell.endpoint(ProxyPool::Single);
    let id = random_id();
    let secret = random_string();

    let mut client_a = Some(Connection::open(&endpoint).await?);
    let mut client_b = None;
    let observed = async {
        let a = client_a.as_ref().unwrap();
        a.simple(&format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{secret}')"
        ))
        .await
        .map_err(|e| format!("seed insert: {e}"))?;
        let own = a
            .simple_value(&format!(
                "SELECT encrypted_text FROM encrypted WHERE id = {id}"
            ))
            .await
            .map_err(|e| format!("seed select: {e}"))?;
        expect_value(own, &secret, "seed select")?;

        let standalone = cell.check == Check::StandaloneMappingCleanup;
        a.simple("BEGIN").await?;
        let scope = if standalone { "LOCAL " } else { "" };
        a.simple(&format!(
            "SET {scope}CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true"
        ))
        .await
        .map_err(|e| format!("client A disable mapping: {e}"))?;

        let ciphertext = a
            .simple_value(&format!(
                "SELECT encrypted_text FROM encrypted WHERE id = {id}"
            ))
            .await?
            .ok_or_else(|| {
                ProbeFailure::Inconclusive("client A's ciphertext row was missing".to_string())
            })?;
        if ciphertext == secret || serde_json::from_str::<serde_json::Value>(&ciphertext).is_err() {
            return Err(ProbeFailure::Inconclusive(
                "client A did not observe stored EQL ciphertext with mapping disabled".to_string(),
            ));
        }

        let prepared_on = a.backend_pid().await?;
        a.simple("COMMIT").await?;
        if standalone {
            a.simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
                .await?;
        }
        if cell.mode == PoolMode::Session {
            client_a.take().unwrap().close().await;
        }

        client_b = Some(Connection::open(&endpoint).await?);
        let b = client_b.as_ref().unwrap();
        require_reused_backend(cell, &prepared_on, b).await?;
        match b
            .simple_value(&format!(
                "SELECT encrypted_text FROM encrypted WHERE id = {id}"
            ))
            .await
        {
            Ok(Some(value)) if value == secret => Ok(()),
            Ok(Some(value)) if value == ciphertext => Err(ProbeFailure::Incompatible(
                Incompatibility::MappingCrossClient,
                "client B received the exact ciphertext client A observed after disabling mapping"
                    .to_string(),
            )),
            Ok(other) => Err(ProbeFailure::Inconclusive(format!(
            "client B returned {other:?}; neither the seeded plaintext nor the stored ciphertext"
        ))),
            Err(e) => Err(format!("client B select failed: {e}").into()),
        }
    }
    .await;
    finish_isolation_probe(
        &endpoint,
        client_a,
        client_b,
        "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false",
        observed,
    )
    .await
}

/// Probe the upstream pooler's protocol directly: a Proxy connection closing is
/// not evidence of why startup failed. A 3.0 control proves the packet and
/// service are valid before the otherwise identical 3.2 startup is attempted.
pub async fn startup_protocol(cell: Cell) -> Result<(), ProbeFailure> {
    let pooler = cell
        .pooler
        .ok_or_else(|| ProbeFailure::Infrastructure("startup probe requires a pooler".into()))?;
    let endpoint = Endpoint {
        port: pooler.port,
        dbname: format!("pg_{}", cell.mode.alias_suffix()),
    };
    startup_at(&endpoint, pooler).await
}

pub async fn startup_at(endpoint: &Endpoint, pooler: Pooler) -> Result<(), ProbeFailure> {
    if pooler.port == super::PGCAT_1_2.port || pooler.port == super::PGCAT_1_2_PS.port {
        return pgcat_startup_refusal(endpoint).await;
    }
    let (kind, _) = startup_response(endpoint, 196608).await?.ok_or_else(|| {
        ProbeFailure::Infrastructure("protocol 3.0 control closed before authentication".into())
    })?;
    if kind != b'R' {
        return Err(ProbeFailure::Infrastructure(
            "protocol 3.0 control did not reach authentication".into(),
        ));
    }
    let (kind, body) = startup_response(endpoint, 196610).await?.ok_or_else(|| {
        ProbeFailure::Inconclusive("startup closed without protocol rejection evidence".into())
    })?;
    match kind {
        b'R' | b'v' => Ok(()),
        b'E' => {
            if !body.ends_with(&[0, 0]) {
                return Err(ProbeFailure::Inconclusive(
                    "unterminated startup error response".into(),
                ));
            }
            let mut message = None;
            let mut code = None;
            let mut rest = body.as_slice();
            while let Some((&field, tail)) = rest.split_first() {
                if field == 0 {
                    if !tail.is_empty() {
                        return Err(ProbeFailure::Inconclusive(
                            "malformed startup error response".into(),
                        ));
                    }
                    break;
                }
                let end = tail.iter().position(|&byte| byte == 0).ok_or_else(|| {
                    ProbeFailure::Inconclusive("unterminated startup error field".into())
                })?;
                let value = std::str::from_utf8(&tail[..end])
                    .map_err(|e| ProbeFailure::Inconclusive(e.to_string()))?;
                match field {
                    b'M' => message = Some(value),
                    b'C' => code = Some(value),
                    _ => {}
                }
                rest = &tail[end + 1..];
            }
            let rejected_header = format!(
                "bad packet header: '{:08x}00030002'",
                startup_body(endpoint, 196610).len() + 4
            );
            match (code, message) {
                (Some("08P01"), Some(message)) if message == rejected_header => Err(ProbeFailure::Incompatible(
                    Incompatibility::Protocol32Refused,
                    format!("3.0 control reached authentication; 3.2 startup rejected by {endpoint:?}: SQLSTATE {}: {}", code.unwrap(), message),
                )),
                _ => Err(ProbeFailure::Unexpected(format!("startup rejected for an unrelated reason: SQLSTATE {code:?}: {message:?}"))),
            }
        }
        other => Err(ProbeFailure::Inconclusive(format!(
            "unexpected startup response tag {other}"
        ))),
    }
}

/// PgCat v1.2.0 drops unsupported startup packets without ErrorResponse:
/// https://github.com/postgresml/pgcat/blob/4a7a6a8e7a78354b889002a4db118a8e2f2d6d79/src/client.rs#L349
/// Its caller logs the failure and drops the socket. Only this pinned fixture
/// uses differential evidence: authenticated 3.0 controls surround each valid
/// 3.2 packet, twice. An unavailable/crashed service, reset or malformed response
/// cannot satisfy the probe. A generic closed connection is never sufficient.
async fn pgcat_startup_refusal(endpoint: &Endpoint) -> Result<(), ProbeFailure> {
    for _ in 0..2 {
        Connection::open(endpoint).await?.close().await;
        match startup_response(endpoint, 196610).await? {
            None => {}
            Some((b'R' | b'v', _)) => return Ok(()),
            Some((kind, _)) => return Err(ProbeFailure::Unexpected(format!(
                "pinned PgCat returned response tag {kind} instead of the documented clean EOF for protocol 3.2"
            ))),
        }
        Connection::open(endpoint).await?.close().await;
    }
    Err(ProbeFailure::Incompatible(
        Incompatibility::Protocol32Refused,
        "pinned PgCat v1.2.0 cleanly refused valid 3.2 startup twice, with successful authenticated 3.0 controls before and after each refusal (no ErrorResponse)".into(),
    ))
}

fn startup_body(endpoint: &Endpoint, version: u32) -> Vec<u8> {
    let mut body = version.to_be_bytes().to_vec();
    body.extend_from_slice(b"user\0cipherstash\0database\0");
    body.extend_from_slice(endpoint.dbname.as_bytes());
    body.extend_from_slice(b"\0\0");
    body
}

async fn startup_response(
    endpoint: &Endpoint,
    version: u32,
) -> Result<Option<(u8, Vec<u8>)>, ProbeFailure> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut socket = tokio::net::TcpStream::connect(("localhost", endpoint.port))
        .await
        .map_err(|e| {
            ProbeFailure::Infrastructure(format!("startup connect to {endpoint:?}: {e}"))
        })?;
    let body = startup_body(endpoint, version);
    socket
        .write_u32((body.len() + 4) as u32)
        .await
        .map_err(|e| ProbeFailure::Infrastructure(e.to_string()))?;
    socket
        .write_all(&body)
        .await
        .map_err(|e| ProbeFailure::Infrastructure(e.to_string()))?;
    let kind = match socket.read_u8().await {
        Ok(kind) => kind,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => {
            return Err(ProbeFailure::Inconclusive(format!(
                "startup socket failed before a response: {e}"
            )))
        }
    };
    let len = socket
        .read_u32()
        .await
        .map_err(|e| ProbeFailure::Inconclusive(e.to_string()))?;
    if !(4..=65536).contains(&len) {
        return Err(ProbeFailure::Inconclusive(format!(
            "invalid startup response length {len}"
        )));
    }
    let mut body = vec![0; (len - 4) as usize];
    socket
        .read_exact(&mut body)
        .await
        .map_err(|e| ProbeFailure::Inconclusive(e.to_string()))?;
    Ok(Some((kind, body)))
}

#[cfg(test)]
mod tests {
    use super::is_missing_keyset;

    #[test]
    fn missing_keyset_requires_the_exact_server_diagnostic() {
        assert!(is_missing_keyset("ERROR: Missing keyset identifer."));
        assert!(is_missing_keyset("FATAL: Missing keyset identifer."));
        assert!(!is_missing_keyset("FATAL: password authentication failed"));
        assert!(!is_missing_keyset(
            "connection closed: Missing keyset identifer."
        ));
    }
}

#[cfg(test)]
mod cleanup_regression_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[derive(Default)]
    struct State {
        secret: String,
        mapping_disabled: bool,
        restore_attempts: usize,
    }

    async fn message(socket: &mut TcpStream, tag: u8, body: &[u8]) {
        socket.write_u8(tag).await.unwrap();
        socket.write_u32((body.len() + 4) as u32).await.unwrap();
        socket.write_all(body).await.unwrap();
    }

    async fn serve(
        mut socket: TcpStream,
        state: Arc<Mutex<State>>,
        connection_id: usize,
        invalid_ciphertext: bool,
        fail_restore: bool,
    ) {
        let length = socket.read_u32().await.unwrap();
        let mut startup = vec![0; (length - 4) as usize];
        socket.read_exact(&mut startup).await.unwrap();
        message(&mut socket, b'R', &0_u32.to_be_bytes()).await;
        message(&mut socket, b'Z', b"I").await;
        while let Ok(tag) = socket.read_u8().await {
            let length = socket.read_u32().await.unwrap();
            let mut body = vec![0; (length - 4) as usize];
            socket.read_exact(&mut body).await.unwrap();
            if tag == b'X' {
                break;
            }
            assert_eq!(tag, b'Q');
            let query = std::str::from_utf8(&body[..body.len() - 1]).unwrap();
            if query == "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false" {
                state.lock().unwrap().restore_attempts += 1;
                if fail_restore {
                    message(
                        &mut socket,
                        b'E',
                        b"SERROR\0CXX000\0Minjected cleanup failure\0\0",
                    )
                    .await;
                    message(&mut socket, b'Z', b"I").await;
                    continue;
                }
            }
            let value = {
                let mut state = state.lock().unwrap();
                if query.starts_with("INSERT INTO encrypted") {
                    state.secret = query.split('\'').nth(1).unwrap().to_string();
                } else if query == "RESET ALL" {
                    state.mapping_disabled = false;
                } else if query.contains("UNSAFE_DISABLE_MAPPING") {
                    state.mapping_disabled = query.ends_with("true");
                }
                if query == "SELECT pg_backend_pid()" {
                    Some(connection_id.to_string())
                } else if query.starts_with("SELECT encrypted_text") {
                    Some(if state.mapping_disabled {
                        if invalid_ciphertext {
                            "not ciphertext".to_string()
                        } else {
                            r#"{"encrypted":true}"#.to_string()
                        }
                    } else {
                        state.secret.clone()
                    })
                } else {
                    None
                }
            };
            if let Some(value) = value {
                let mut description = 1_u16.to_be_bytes().to_vec();
                description.extend_from_slice(b"value\0");
                description.extend_from_slice(&0_u32.to_be_bytes());
                description.extend_from_slice(&0_u16.to_be_bytes());
                description.extend_from_slice(&25_u32.to_be_bytes());
                description.extend_from_slice(&(-1_i16).to_be_bytes());
                description.extend_from_slice(&(-1_i32).to_be_bytes());
                description.extend_from_slice(&0_u16.to_be_bytes());
                message(&mut socket, b'T', &description).await;
                let mut row = 1_u16.to_be_bytes().to_vec();
                row.extend_from_slice(&(value.len() as u32).to_be_bytes());
                row.extend_from_slice(value.as_bytes());
                message(&mut socket, b'D', &row).await;
            }
            message(&mut socket, b'C', b"SELECT 1\0").await;
            message(&mut socket, b'Z', b"I").await;
        }
    }

    async fn assert_mapping_restored(invalid_ciphertext: bool, fail_restore: bool) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            let mut connection_id = 0;
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                connection_id += 1;
                connections.spawn(serve(
                    socket,
                    server_state.clone(),
                    connection_id,
                    invalid_ciphertext,
                    fail_restore,
                ));
            }
        });
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            disable_mapping_isolation(Cell {
                pooler: Some(Pooler {
                    name: "cleanup mock",
                    port,
                    proxy_in_front_port: port,
                }),
                topology: super::super::Topology::PoolerInFrontOfProxy,
                mode: PoolMode::Transaction,
                check: Check::DisableMappingIsolation,
            }),
        )
        .await
        .unwrap();
        server.abort();
        let error = result.unwrap_err().to_string();
        let expected = if invalid_ciphertext {
            "did not observe stored EQL ciphertext"
        } else {
            "isolation was not tested on a reused connection"
        };
        assert!(
            error.contains(expected),
            "original probe failure lost: {error}"
        );
        if fail_restore {
            assert!(error.contains("restore client A settings: ERROR: injected cleanup failure"));
            assert!(error.contains("restore client B settings: ERROR: injected cleanup failure"));
            assert!(error.contains("restore probe pool: ERROR: injected cleanup failure"));
            assert_eq!(state.lock().unwrap().restore_attempts, 3);
            return;
        }
        assert!(
            !state.lock().unwrap().mapping_disabled,
            "failed probe left mapping disabled for the next matrix cell"
        );
    }

    #[tokio::test]
    async fn restores_mapping_after_ciphertext_observation_fails() {
        assert_mapping_restored(true, false).await;
    }

    #[tokio::test]
    async fn restores_mapping_after_backend_reuse_check_fails() {
        assert_mapping_restored(false, false).await;
    }
    #[tokio::test]
    async fn retains_probe_failure_and_attempts_every_cleanup_after_cleanup_errors() {
        assert_mapping_restored(false, true).await;
    }
}
