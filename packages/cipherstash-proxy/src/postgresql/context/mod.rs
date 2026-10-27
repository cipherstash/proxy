pub mod column;
pub mod phase_timing;
pub mod portal;
pub mod settings;
pub mod statement;
pub mod statement_metadata;
pub use self::{
    phase_timing::PhaseTiming,
    portal::Portal,
    settings::{ConnectionSettings, EffectiveSettings, SettingChange, SettingsCommand},
    statement::Statement,
};
use super::{
    column_mapper::ColumnMapper,
    messages::{describe::Describe, Name, Target},
    Column,
};
use crate::{
    config::TandemConfig,
    error::{EncryptError, Error},
    log::{CONTEXT, SLOW_STATEMENTS},
    prometheus::{
        SLOW_STATEMENTS_TOTAL, STATEMENTS_EXECUTION_DURATION_SECONDS,
        STATEMENTS_SESSION_DURATION_SECONDS,
    },
    proxy::{EncryptConfig, EncryptionService, ReloadCommand, ReloadSender},
};
use bytes::BytesMut;
use cipherstash_client::IdentifiedBy;
use eql_mapper::{Schema, TableResolver};
use metrics::{counter, histogram};
use serde_json::json;
pub use statement_metadata::StatementMetadata;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, Notify};
use tracing::{debug, error, warn};

type DescribeQueue = Queue<Describe>;
type ExecuteQueue = Queue<ExecuteContext>;
type SessionMetricsQueue = Queue<SessionMetricsContext>;
type PortalQueue = Queue<Arc<Portal>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

#[derive(Clone, Debug, PartialEq)]
pub struct KeysetIdentifier(pub IdentifiedBy);

impl std::fmt::Display for KeysetIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone)]
pub struct Context<T>
where
    T: EncryptionService,
{
    pub client_id: i32,
    config: Arc<TandemConfig>,
    encrypt_config: Arc<EncryptConfig>,
    encryption: T,
    reload_sender: ReloadSender,
    column_mapper: ColumnMapper,
    statements: Arc<RwLock<HashMap<Name, Arc<Statement>>>>,
    statement_sessions: Arc<RwLock<HashMap<Name, SessionId>>>,
    portals: Arc<RwLock<HashMap<Name, PortalQueue>>>,
    describe: Arc<RwLock<DescribeQueue>>,
    execute: Arc<RwLock<ExecuteQueue>>,
    schema_changed: Arc<RwLock<bool>>,
    session_metrics: Arc<RwLock<SessionMetricsQueue>>,
    table_resolver: Arc<TableResolver>,
    /// The `SET CIPHERSTASH.*` settings PostgreSQL has confirmed, and the commands it has not
    /// answered yet. Shared by the frontend, which queues commands, and the backend, which
    /// applies them as PostgreSQL answers.
    settings: Arc<Mutex<SettingsState>>,
    /// Notified on every `ReadyForQuery`, see [`Context::settings_barrier`]
    batch_answered: Arc<Notify>,
    session_id_counter: Arc<AtomicU64>,
    sync_generation: Arc<AtomicU64>,
    ready_generation: Arc<AtomicU64>,
    /// `sync_generation` when the last `Execute` or `Query` was forwarded
    execution_generation: Arc<AtomicU64>,
    /// The client is sending `COPY ... FROM STDIN` data
    copy_in: Arc<AtomicBool>,
    /// PostgreSQL sent a `CopyInResponse` that no copy message has answered yet
    copy_in_response: Arc<AtomicBool>,
    /// Number of `Parse` messages seen, see [`Context::set_statement_session`]
    parse_sequence: Arc<AtomicU64>,
    /// The `parse_sequence` value assigned to each statement by its `Parse`
    statement_sequences: Arc<RwLock<HashMap<Name, u64>>>,
    pending_deallocations: Arc<RwLock<VecDeque<PendingDeallocation>>>,
    /// Statements whose SQL is a `DEALLOCATE` or `DISCARD ALL`
    statement_deallocations: Arc<RwLock<HashMap<Name, Deallocation>>>,
    /// Portals bound from one of those statements, with the generation they were bound in
    portal_deallocations: Arc<RwLock<HashMap<Name, (u64, Deallocation)>>>,
    /// Statements that change settings or end a transaction, with their settings command
    statement_settings: Arc<RwLock<HashMap<Name, SettingsCommand>>>,
    /// Portals bound from one of those statements, with the generation they were bound in
    portal_settings: Arc<RwLock<HashMap<Name, (u64, SettingsCommand)>>>,
    /// Statements Proxy maps, with the settings they were prepared under
    prepared_settings: Arc<RwLock<HashMap<Name, PreparedSettings>>>,
    /// Portals bound from one of those statements, with the generation they were bound in
    bound_settings: Arc<RwLock<HashMap<Name, (u64, BoundSettings)>>>,
    /// The errors of messages Proxy refused, see [`Context::refuse`]
    refusals: Arc<RwLock<VecDeque<Refusal>>>,
}

/// A message Proxy refused, forwarded as a message that names an object that does not exist.
#[derive(Debug)]
struct Refusal {
    /// Generation of the batch it was sent in, see [`Context::sync_generation`]
    generation: u64,
    /// The name of the object that does not exist, which PostgreSQL's error names
    name: String,
    /// Proxy's error, sent to the client in place of PostgreSQL's
    response: BytesMut,
}

/// The settings a statement Proxy maps was prepared under.
#[derive(Clone, Debug)]
struct PreparedSettings {
    settings: EffectiveSettings,
    /// The statement holds literals encrypted under the keyset it was prepared with
    encrypted_literals: bool,
}

/// The settings a portal of a statement Proxy maps was bound under.
#[derive(Clone, Debug)]
struct BoundSettings {
    settings: EffectiveSettings,
    /// The portal holds values encrypted under the keyset it was bound with
    encrypted_values: bool,
}

/// The connection's settings, and the settings commands waiting for PostgreSQL's answer.
#[derive(Debug, Default)]
struct SettingsState {
    confirmed: ConnectionSettings,
    pending: VecDeque<PendingSettings>,
    /// [`SettingsState::projected`], kept while it stays valid, so a long pipeline does not
    /// replay every pending command for each message
    projected: Option<ConnectionSettings>,
    /// Generation of the last batch with a command that may change the settings, see
    /// [`Context::settings_barrier`]
    barrier: Option<u64>,
}

impl SettingsState {
    /// The confirmed settings, with every pending command assumed to succeed
    fn projected(&mut self) -> ConnectionSettings {
        let confirmed = &self.confirmed;
        let pending = &self.pending;
        self.projected
            .get_or_insert_with(|| {
                let mut settings = confirmed.clone();
                for pending in pending {
                    settings.project(&pending.command);
                }
                settings
            })
            .clone()
    }

    fn push(&mut self, pending: PendingSettings) {
        if !matches!(pending.command, SettingsCommand::Other) {
            let mut projected = self.projected();
            if pending.command.may_change_effective(&projected) {
                self.barrier = Some(pending.generation);
            }
            projected.project(&pending.command);
            self.projected = Some(projected);
        }
        self.pending.push_back(pending);
    }

    /// The pending entry at the front, applied to the confirmed settings.
    ///
    /// The projection stays valid when that entry changes no setting, as it was projected as
    /// succeeding.
    fn pop_front(&mut self) -> Option<PendingSettings> {
        let pending = self.pending.pop_front()?;
        if !matches!(pending.command, SettingsCommand::Other) {
            self.projected = None;
        }
        Some(pending)
    }

    /// Change the confirmed settings in a way the projection did not assume
    fn confirmed_mut(&mut self) -> &mut ConnectionSettings {
        self.projected = None;
        &mut self.confirmed
    }
}

/// A settings command forwarded to PostgreSQL and waiting for its response.
///
/// Every statement of a simple `Query` that contains a settings command is queued, and every
/// `Execute`, as PostgreSQL answers each with its own `CommandComplete` (or
/// `EmptyQueryResponse` or `PortalSuspended`), in order.
#[derive(Clone, Debug)]
struct PendingSettings {
    /// Generation of the batch it was sent in, see [`Context::sync_generation`]
    generation: u64,
    command: SettingsCommand,
}

/// A prepared-statement removal requested with SQL rather than a protocol `Close`.
///
/// Applied only once PostgreSQL confirms the command with a `CommandComplete`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deallocation {
    /// `DEALLOCATE name`
    Statement(Name),
    /// `DEALLOCATE ALL` or `DISCARD ALL`
    ///
    /// `DISCARD ALL` also closes portals, but it cannot run inside a transaction block, so any
    /// earlier portals are already gone and are discarded by [`Context::ready_for_query`].
    AllStatements,
}

/// A [`Deallocation`] forwarded to PostgreSQL and waiting for its `CommandComplete`.
#[derive(Clone, Debug)]
struct PendingDeallocation {
    /// Generation of the batch it was sent in, see [`Context::sync_generation`]
    generation: u64,
    /// Only statements with a parse sequence up to this value existed when it was sent
    parsed_before: u64,
    deallocation: Deallocation,
}

/// Context for tracking an in-flight Execute operation.
///
/// Timing data is accumulated here during backend message processing because
/// the backend operates on the execute queue rather than having direct access
/// to the session metrics queue. On completion via `complete_execution()`,
/// timing is transferred to the associated SessionMetricsContext.
#[derive(Clone, Debug)]
pub struct ExecuteContext {
    name: Name,
    start: Instant,
    session_id: Option<SessionId>,
    /// Server wait duration (time to first response byte).
    /// Accumulated here during execution, transferred to SessionMetricsContext on completion.
    server_wait_duration: Option<Duration>,
    /// Server response duration (time spent receiving response data after first byte).
    /// Accumulated here during execution, transferred to SessionMetricsContext on completion.
    server_response_duration: Duration,
}

impl ExecuteContext {
    fn new(name: Name, session_id: Option<SessionId>) -> ExecuteContext {
        ExecuteContext {
            name,
            start: Instant::now(),
            session_id,
            server_wait_duration: None,
            server_response_duration: Duration::from_secs(0),
        }
    }

    fn duration(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }

    fn record_server_wait_or_add_response(&mut self, duration: Duration) {
        if self.server_wait_duration.is_none() {
            self.server_wait_duration = Some(duration);
        } else {
            self.server_response_duration += duration;
        }
    }

    fn server_wait_duration(&self) -> Option<Duration> {
        self.server_wait_duration
    }

    fn server_response_duration(&self) -> Duration {
        self.server_response_duration
    }

    fn session_id(&self) -> Option<SessionId> {
        self.session_id
    }
}

#[derive(Clone, Debug)]
pub struct SessionMetricsContext {
    id: SessionId,
    start: Instant,
    pub phase_timing: PhaseTiming,
    pub metadata: StatementMetadata,
}

impl SessionMetricsContext {
    fn new(id: SessionId) -> SessionMetricsContext {
        SessionMetricsContext {
            id,
            start: Instant::now(),
            phase_timing: PhaseTiming::new(),
            metadata: StatementMetadata::new(),
        }
    }

    fn duration(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }

    fn id(&self) -> SessionId {
        self.id
    }
}

#[cfg(test)]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StateSizes {
    pub statements: usize,
    pub statement_sessions: usize,
    pub statement_sequences: usize,
    pub portals: usize,
    pub describe: usize,
    pub execute: usize,
    pub session_metrics: usize,
    pub pending_deallocations: usize,
    pub statement_deallocations: usize,
    pub portal_deallocations: usize,
    pub pending_settings: usize,
    pub statement_settings: usize,
    pub portal_settings: usize,
    pub prepared_settings: usize,
    pub bound_settings: usize,
    pub refusals: usize,
}

#[derive(Clone, Debug)]
pub struct Queue<T> {
    pub queue: VecDeque<T>,
    /// Generation of each entry in `queue`, see [`Context::sync_generation`]
    generations: VecDeque<u64>,
}

impl<T> Context<T>
where
    T: EncryptionService,
{
    pub fn new(
        client_id: i32,
        config: Arc<TandemConfig>,
        encrypt_config: Arc<EncryptConfig>,
        schema: Arc<Schema>,
        encryption: T,
        reload_sender: ReloadSender,
    ) -> Context<T> {
        let column_mapper = ColumnMapper::new(encrypt_config.clone());

        Context {
            statements: Arc::new(RwLock::new(HashMap::new())),
            statement_sessions: Arc::new(RwLock::new(HashMap::new())),
            portals: Arc::new(RwLock::new(HashMap::new())),
            describe: Arc::new(RwLock::from(Queue::new())),
            execute: Arc::new(RwLock::from(Queue::new())),
            schema_changed: Arc::new(RwLock::from(false)),
            session_metrics: Arc::new(RwLock::from(Queue::new())),
            table_resolver: Arc::new(TableResolver::new_editable(schema)),
            client_id,
            config,
            encrypt_config,
            column_mapper,
            encryption,
            reload_sender,
            settings: Arc::new(Mutex::new(SettingsState::default())),
            batch_answered: Arc::new(Notify::new()),
            session_id_counter: Arc::new(AtomicU64::new(1)),
            // The StartupMessage is answered by the first ReadyForQuery
            sync_generation: Arc::new(AtomicU64::new(1)),
            ready_generation: Arc::new(AtomicU64::new(0)),
            execution_generation: Arc::new(AtomicU64::new(1)),
            copy_in: Arc::new(AtomicBool::new(false)),
            copy_in_response: Arc::new(AtomicBool::new(false)),
            parse_sequence: Arc::new(AtomicU64::new(0)),
            statement_sequences: Arc::new(RwLock::new(HashMap::new())),
            pending_deallocations: Arc::new(RwLock::new(VecDeque::new())),
            statement_deallocations: Arc::new(RwLock::new(HashMap::new())),
            portal_deallocations: Arc::new(RwLock::new(HashMap::new())),
            statement_settings: Arc::new(RwLock::new(HashMap::new())),
            portal_settings: Arc::new(RwLock::new(HashMap::new())),
            prepared_settings: Arc::new(RwLock::new(HashMap::new())),
            bound_settings: Arc::new(RwLock::new(HashMap::new())),
            refusals: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    pub fn set_describe(&mut self, describe: Describe) {
        debug!(target: CONTEXT, client_id = self.client_id, describe = ?describe);
        let generation = self.sync_generation();
        let _ = self
            .describe
            .write()
            .map(|mut queue| queue.add(describe, generation));
    }
    ///
    /// Marks the current Describe as complete
    /// Removes the Describe from the Queue
    ///
    pub fn complete_describe(&mut self) {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Describe complete");
        let _ = self.describe.write().map(|mut queue| queue.complete());
    }

    pub fn start_session(&mut self) -> SessionId {
        let id = SessionId(self.session_id_counter.fetch_add(1, Ordering::Relaxed));
        let ctx = SessionMetricsContext::new(id);
        let generation = self.sync_generation();
        let _ = self
            .session_metrics
            .write()
            .map(|mut queue| queue.add(ctx, generation));
        id
    }

    pub fn finish_session(&mut self) {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Session Metrics finished");

        if let Some(session) = self.get_session_metrics() {
            let duration = session.duration();
            let metadata = &session.metadata;

            // Get labels for metrics
            let statement_type = metadata
                .statement_type
                .map(|t| t.as_label())
                .unwrap_or("unknown");
            let protocol = metadata.protocol.map(|p| p.as_label()).unwrap_or("unknown");
            let mapped = if metadata.encrypted { "true" } else { "false" };
            let multi_statement = if metadata.multi_statement {
                "true"
            } else {
                "false"
            };

            // Record with labels
            histogram!(
                STATEMENTS_SESSION_DURATION_SECONDS,
                "statement_type" => statement_type,
                "protocol" => protocol,
                "mapped" => mapped,
                "multi_statement" => multi_statement
            )
            .record(duration);

            // Log slow statements when enabled
            if self.config.slow_statements_enabled()
                && duration > self.config.slow_statement_min_duration()
            {
                let timing = &session.phase_timing;

                // Increment slow statements counter
                counter!(SLOW_STATEMENTS_TOTAL).increment(1);

                let breakdown = json!({
                    "parse_ms": timing.parse_duration.map(|d| d.as_millis()),
                    "encrypt_ms": timing.encrypt_duration.map(|d| d.as_millis()),
                    "server_write_ms": timing.server_write_duration.map(|d| d.as_millis()),
                    "server_wait_ms": timing.server_wait_duration.map(|d| d.as_millis()),
                    "server_response_ms": timing.server_response_duration.map(|d| d.as_millis()),
                    "client_write_ms": timing.client_write_duration.map(|d| d.as_millis()),
                    "decrypt_ms": timing.decrypt_duration.map(|d| d.as_millis()),
                });

                warn!(
                    target: SLOW_STATEMENTS,
                    client_id = self.client_id,
                    duration_ms = duration.as_millis() as u64,
                    statement_type = statement_type,
                    protocol = protocol,
                    encrypted = metadata.encrypted,
                    multi_statement = metadata.multi_statement,
                    encrypted_values_count = metadata.encrypted_values_count,
                    param_bytes = metadata.param_bytes,
                    query_fingerprint = ?metadata.query_fingerprint,
                    keyset_id = ?self.keyset_identifier(),
                    mapping_disabled = self.mapping_disabled(),
                    breakdown = %breakdown,
                    msg = "Slow statement detected"
                );
            }
        }

        let _ = self
            .session_metrics
            .write()
            .map(|mut queue| queue.complete());
    }

    pub fn set_execute(&mut self, name: Name, session_id: Option<SessionId>) {
        debug!(target: CONTEXT, client_id = self.client_id, execute = ?name);

        let ctx = ExecuteContext::new(name, session_id);

        let generation = self.sync_generation();
        let _ = self
            .execute
            .write()
            .map(|mut queue| queue.add(ctx, generation));
    }

    /// Set execute state for portal, looking up session ID internally.
    pub fn set_execute_for_portal(&mut self, name: Name) {
        let command = self.portal_settings_command(&name);
        self.execute_portal(name, command);
    }

    /// The settings command the portal `name` runs.
    pub fn portal_settings_command(&self, name: &Name) -> SettingsCommand {
        self.portal_settings
            .read()
            .ok()
            .and_then(|portals| portals.get(name).map(|(_, command)| command.clone()))
            .unwrap_or(SettingsCommand::Other)
    }

    /// Set execute state for the portal `name`, which runs the settings command `command`.
    pub fn execute_portal(&mut self, name: Name, command: SettingsCommand) {
        let session_id = self.get_portal_session_id(&name);
        // A portal running `DEALLOCATE` or `DISCARD ALL` through the extended protocol
        let deallocation = self
            .portal_deallocations
            .read()
            .ok()
            .and_then(|portals| portals.get(&name).map(|(_, d)| d.clone()));
        if let Some(deallocation) = deallocation {
            self.add_deallocations(vec![deallocation]);
        }
        // Every Execute is queued, so each settings command waits for its own response
        self.queue_settings_commands(vec![command]);
        self.set_execute(name, session_id);
    }

    /// Number of entries currently queued in the `execute` queue.
    ///
    /// Test-only accessor used by the BUG-300 regression tests to assert the
    /// per-connection queues are drained (and do not grow unbounded).
    #[cfg(test)]
    pub(crate) fn execute_queue_len(&self) -> usize {
        self.execute.read().unwrap().queue.len()
    }

    /// Number of entries currently queued in the `session_metrics` queue.
    ///
    /// Test-only accessor used by the BUG-300 regression tests to assert the
    /// per-connection queues are drained (and do not grow unbounded).
    #[cfg(test)]
    pub(crate) fn session_metrics_queue_len(&self) -> usize {
        self.session_metrics.read().unwrap().queue.len()
    }

    /// Size of every per-connection protocol state collection.
    ///
    /// Test-only accessor used to assert per-connection state stays bounded.
    #[cfg(test)]
    pub(crate) fn state_sizes(&self) -> StateSizes {
        StateSizes {
            statements: self.statements.read().unwrap().len(),
            statement_sessions: self.statement_sessions.read().unwrap().len(),
            statement_sequences: self.statement_sequences.read().unwrap().len(),
            portals: self.portals.read().unwrap().len(),
            describe: self.describe.read().unwrap().queue.len(),
            execute: self.execute.read().unwrap().queue.len(),
            session_metrics: self.session_metrics.read().unwrap().queue.len(),
            pending_deallocations: self.pending_deallocations.read().unwrap().len(),
            statement_deallocations: self.statement_deallocations.read().unwrap().len(),
            portal_deallocations: self.portal_deallocations.read().unwrap().len(),
            pending_settings: self.settings.lock().unwrap().pending.len(),
            statement_settings: self.statement_settings.read().unwrap().len(),
            portal_settings: self.portal_settings.read().unwrap().len(),
            prepared_settings: self.prepared_settings.read().unwrap().len(),
            bound_settings: self.bound_settings.read().unwrap().len(),
            refusals: self.refusals.read().unwrap().len(),
        }
    }

    /// Marks the current Execution as Complete.
    ///
    /// Transfers accumulated timing data from ExecuteContext to SessionMetricsContext.phase_timing:
    /// - `server_wait_duration` (time to first response byte) is recorded to the session
    /// - `server_response_duration` (time receiving response data) is added to the session
    ///
    /// This two-phase timing pattern exists because the backend operates on the execute queue
    /// rather than having direct access to the session. Timing is accumulated in ExecuteContext
    /// during message processing, then transferred to the correct SessionMetricsContext here.
    ///
    /// If the associated portal is Unnamed, it is closed.
    ///
    /// From the PostgreSQL Extended Query docs:
    ///     If successfully created, a named portal object lasts till the end of the current transaction, unless explicitly destroyed.
    ///     An unnamed portal is destroyed at the end of the transaction, or as soon as the next Bind statement specifying the unnamed portal as destination is issued
    ///
    /// https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-EXT-QUERY
    ///
    /// An Execute queued by a later, pipelined batch is left alone: an `ErrorResponse` for a
    /// batch that never reached its Execute must not take the next batch's.
    pub fn complete_execution(&mut self) {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Execute complete");

        let answering = self.ready_generation();
        let Some(generation) = self
            .execute
            .read()
            .ok()
            .and_then(|queue| queue.next_generation())
            .filter(|g| *g <= answering)
        else {
            return;
        };

        if let Some(execute) = self.get_execute() {
            if let Some(session_id) = execute.session_id() {
                if let Some(wait) = execute.server_wait_duration() {
                    self.record_server_wait_duration(session_id, wait);
                }
                let response = execute.server_response_duration();
                if !response.is_zero() {
                    self.add_server_response_duration(session_id, response);
                }
            }

            // Get labels from current session metadata
            let (statement_type, protocol, mapped, multi_statement) =
                if let Some(session) = self.get_session_metrics() {
                    let metadata = &session.metadata;
                    (
                        metadata
                            .statement_type
                            .map(|t| t.as_label())
                            .unwrap_or("unknown"),
                        metadata.protocol.map(|p| p.as_label()).unwrap_or("unknown"),
                        if metadata.encrypted { "true" } else { "false" },
                        if metadata.multi_statement {
                            "true"
                        } else {
                            "false"
                        },
                    )
                } else {
                    ("unknown", "unknown", "false", "false")
                };

            histogram!(
                STATEMENTS_EXECUTION_DURATION_SECONDS,
                "statement_type" => statement_type,
                "protocol" => protocol,
                "mapped" => mapped,
                "multi_statement" => multi_statement
            )
            .record(execute.duration());

            if execute.name.is_unnamed() {
                self.close_portal_bound_by(&execute.name, generation);
            }
        }

        let _ = self.execute.write().map(|mut queue| queue.complete());
    }

    pub fn add_statement(&mut self, name: Name, statement: Statement) {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);
        let _ = self
            .statements
            .write()
            .map(|mut guarded| guarded.insert(name, Arc::new(statement)));
    }

    pub fn close_statement(&mut self, name: &Name) {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);

        let _ = self
            .statements
            .write()
            .map(|mut guarded| guarded.remove(name));

        let _ = self
            .statement_sessions
            .write()
            .map(|mut guarded| guarded.remove(name));

        let _ = self
            .statement_sequences
            .write()
            .map(|mut guarded| guarded.remove(name));

        let _ = self
            .statement_deallocations
            .write()
            .map(|mut guarded| guarded.remove(name));

        let _ = self
            .statement_settings
            .write()
            .map(|mut guarded| guarded.remove(name));

        let _ = self
            .prepared_settings
            .write()
            .map(|mut guarded| guarded.remove(name));
    }

    /// Record whether the SQL parsed as statement `name` is a `DEALLOCATE` or `DISCARD ALL`.
    pub fn set_statement_deallocation(&mut self, name: Name, deallocation: Option<Deallocation>) {
        let _ = self.statement_deallocations.write().map(|mut guarded| {
            match deallocation {
                Some(deallocation) => guarded.insert(name, deallocation),
                None => guarded.remove(&name),
            };
        });
    }

    /// Carry the deallocation of `statement`, if any, onto the portal bound from it.
    pub fn bind_portal_deallocation(&mut self, portal: &Name, statement: &Name) {
        let deallocation = self
            .statement_deallocations
            .read()
            .ok()
            .and_then(|statements| statements.get(statement).cloned());
        let generation = self.sync_generation();
        let _ = self.portal_deallocations.write().map(|mut guarded| {
            match deallocation {
                Some(deallocation) => guarded.insert(portal.clone(), (generation, deallocation)),
                None => guarded.remove(portal),
            };
        });
    }

    /// Record the settings command the SQL parsed as statement `name` runs, if any, so that
    /// each execution of it is applied when PostgreSQL confirms it.
    pub fn set_statement_settings(&mut self, name: Name, command: SettingsCommand) {
        let _ = self.statement_settings.write().map(|mut guarded| {
            match command {
                SettingsCommand::Other => guarded.remove(&name),
                command => guarded.insert(name, command),
            };
        });
    }

    /// Carry the settings command of `statement`, if any, onto the portal bound from it.
    pub fn bind_portal_settings(&mut self, portal: &Name, statement: &Name) {
        let command = self
            .statement_settings
            .read()
            .ok()
            .and_then(|statements| statements.get(statement).cloned());
        let generation = self.sync_generation();
        let _ = self.portal_settings.write().map(|mut guarded| {
            match command {
                Some(command) => guarded.insert(portal.clone(), (generation, command)),
                None => guarded.remove(portal),
            };
        });
    }

    /// Record the settings statement `name` is prepared under, if it is a statement Proxy maps.
    ///
    /// A statement prepared under other settings than it is bound under is refused, see
    /// [`Context::bind_prepared_settings`].
    pub fn set_prepared_settings(&mut self, name: Name, settings: Option<EffectiveSettings>) {
        let _ = self.prepared_settings.write().map(|mut guarded| {
            match settings {
                Some(settings) => guarded.insert(
                    name,
                    PreparedSettings {
                        settings,
                        encrypted_literals: false,
                    },
                ),
                None => guarded.remove(&name),
            };
        });
    }

    /// Record that statement `name` holds literals encrypted under the keyset it was prepared
    /// with.
    pub fn set_prepared_encrypted_literals(&mut self, name: &Name) {
        let _ = self.prepared_settings.write().map(|mut guarded| {
            if let Some(prepared) = guarded.get_mut(name) {
                prepared.encrypted_literals = true;
            }
        });
    }

    /// Check that `statement` can be bound to `portal` under `settings`, and record the settings
    /// the portal is bound under.
    ///
    /// Refuses a statement Proxy maps if mapping has changed since it was prepared, or if the
    /// keyset has changed and it holds encrypted literals.
    pub fn bind_prepared_settings(
        &mut self,
        portal: &Name,
        statement: &Name,
        settings: &ConnectionSettings,
        encrypted_values: bool,
    ) -> Result<(), Error> {
        let prepared = self
            .prepared_settings
            .read()
            .ok()
            .and_then(|statements| statements.get(statement).cloned());
        let current = EffectiveSettings::new(settings);
        let bound = match prepared {
            Some(prepared) => {
                if let Some(refusal) = prepared
                    .settings
                    .refusal(&current, prepared.encrypted_literals)
                {
                    return Err(self.prepared_settings_changed(refusal));
                }
                Some(BoundSettings {
                    settings: current,
                    encrypted_values: encrypted_values || prepared.encrypted_literals,
                })
            }
            None => None,
        };
        let generation = self.sync_generation();
        let _ = self.bound_settings.write().map(|mut guarded| {
            match bound {
                Some(bound) => guarded.insert(portal.clone(), (generation, bound)),
                None => guarded.remove(portal),
            };
        });
        Ok(())
    }

    /// Record the error of a message Proxy refused, which is forwarded as a message that names
    /// `name`, an object that does not exist, so the client gets `response` in place of
    /// PostgreSQL's error.
    pub fn refuse(&mut self, name: String, response: BytesMut) {
        let generation = self.sync_generation();
        let _ = self.refusals.write().map(|mut refusals| {
            refusals.push_back(Refusal {
                generation,
                name,
                response,
            })
        });
    }

    /// The error to send the client in place of `error`, the `ErrorResponse` PostgreSQL is
    /// answering the current batch with, if it rejects a message Proxy refused.
    ///
    /// PostgreSQL skips the rest of a batch after an error, so an error that does not name the
    /// refused message's object is from an earlier message, and reaches the client as it is.
    pub fn take_refusal(&mut self, error: &[u8]) -> Option<BytesMut> {
        let answering = self.ready_generation();
        let mut refusals = self.refusals.write().ok()?;
        while refusals
            .front()
            .is_some_and(|refusal| refusal.generation < answering)
        {
            refusals.pop_front();
        }
        if !refusals
            .front()
            .is_some_and(|refusal| refusal.generation == answering)
        {
            return None;
        }
        let refusal = refusals.pop_front()?;
        let name = refusal.name.as_bytes();
        error
            .windows(name.len())
            .any(|window| window == name)
            .then_some(refusal.response)
    }

    /// Check that `portal` can be executed under `settings`.
    ///
    /// Refuses a portal of a statement Proxy maps if mapping has changed since it was bound, or
    /// if the keyset has changed and it holds encrypted values.
    pub fn check_bound_settings(
        &self,
        portal: &Name,
        settings: &ConnectionSettings,
    ) -> Result<(), Error> {
        let bound = self
            .bound_settings
            .read()
            .ok()
            .and_then(|portals| portals.get(portal).map(|(_, bound)| bound.clone()));
        let refusal = bound.and_then(|bound| {
            bound
                .settings
                .refusal(&EffectiveSettings::new(settings), bound.encrypted_values)
        });
        match refusal {
            Some(refusal) => Err(self.prepared_settings_changed(refusal)),
            None => Ok(()),
        }
    }

    fn prepared_settings_changed(&self, refusal: settings::SettingsRefusal) -> Error {
        warn!(
            target: CONTEXT,
            client_id = self.client_id,
            ?refusal,
            msg = "Prepared statement refused after the connection settings changed"
        );
        crate::error::MappingError::PreparedSettingsChanged.into()
    }

    /// Close both statement and its associated portal.
    pub fn close_statement_and_portal(&mut self, name: &Name) {
        self.close_portal(name);
        self.close_statement(name);
    }

    pub fn add_portal(&mut self, name: Name, portal: Portal) {
        debug!(target: CONTEXT, client_id = self.client_id, name = ?name, portal = ?portal);
        let generation = self.sync_generation();
        let _ = self.portals.write().map(|mut portals| {
            portals
                .entry(name)
                .or_insert_with(Queue::new)
                .add(Arc::new(portal), generation);
        });
    }

    pub fn get_statement(&self, name: &Name) -> Option<Arc<Statement>> {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);
        let statements = self.statements.read().ok()?;
        statements.get(name).cloned()
    }

    /// Record the session of the statement prepared by a `Parse` that is about to be forwarded.
    ///
    /// Also gives the statement the next parse sequence number, so a SQL deallocation removes
    /// it only if the statement was parsed before the deallocation was sent.
    pub fn set_statement_session(&mut self, name: Name, session_id: SessionId) {
        let sequence = self.parse_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self
            .statement_sequences
            .write()
            .map(|mut sequences| sequences.insert(name.clone(), sequence));
        let _ = self
            .statement_sessions
            .write()
            .map(|mut sessions| sessions.insert(name, session_id));
    }

    pub fn get_statement_session(&self, name: &Name) -> Option<SessionId> {
        let sessions = self.statement_sessions.read().ok()?;
        sessions.get(name).copied()
    }

    /// Get session for statement, falling back to latest session with warning log.
    pub fn get_statement_session_or_latest(&self, name: &Name) -> Option<SessionId> {
        if let Some(id) = self.get_statement_session(name) {
            return Some(id);
        }

        let fallback = self.latest_session_id();
        if fallback.is_some() {
            warn!(
                target: CONTEXT,
                client_id = self.client_id,
                prepared_statement = %name.as_str(),
                msg = "Session lookup failed for prepared statement, using latest session"
            );
        }
        fallback
    }

    ///
    /// Close the portal identified by `name`
    /// Portal is removed from  queue
    ///
    pub fn close_portal(&mut self, name: &Name) {
        self.close_portal_bound_by(name, u64::MAX);
    }

    /// Close the portal `name` for an execution of generation `generation`.
    ///
    /// The SQL deallocation and settings command of a portal bound in a later batch are kept: a
    /// client can bind the unnamed portal for its next batch before PostgreSQL has finished
    /// executing the current one.
    fn close_portal_bound_by(&mut self, name: &Name, generation: u64) {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Close Portal", name = ?name);
        let _ = self.portals.write().map(|mut portals| {
            if let Some(queue) = portals.get_mut(name) {
                queue.complete();
                if queue.is_empty() {
                    portals.remove(name);
                }
            }
        });
        let _ = self.portal_deallocations.write().map(|mut guarded| {
            if guarded
                .get(name)
                .is_some_and(|(bound, _)| *bound <= generation)
            {
                guarded.remove(name);
            }
        });
        let _ = self.portal_settings.write().map(|mut guarded| {
            if guarded
                .get(name)
                .is_some_and(|(bound, _)| *bound <= generation)
            {
                guarded.remove(name);
            }
        });
        let _ = self.bound_settings.write().map(|mut guarded| {
            if guarded
                .get(name)
                .is_some_and(|(bound, _)| *bound <= generation)
            {
                guarded.remove(name);
            }
        });
    }

    /// The generation assigned to protocol state created now.
    ///
    /// A generation is the number of `ReadyForQuery`-producing messages (the `StartupMessage`,
    /// then `Sync`, `Query` and `FunctionCall`) forwarded to PostgreSQL so far, so all state
    /// created for one batch shares a generation and later batches have higher ones.
    pub fn sync_generation(&self) -> u64 {
        self.sync_generation.load(Ordering::Acquire)
    }

    /// Record that a `ReadyForQuery`-producing message is about to be forwarded to PostgreSQL.
    ///
    /// Must be called before the message is written, so the matching `ReadyForQuery` can never
    /// be read first.
    pub fn sync_sent(&self) {
        if self.copy_in.load(Ordering::Acquire) {
            return;
        }
        self.sync_generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Record that an `Execute` or `Query` is about to be forwarded, after any [`Self::sync_sent`]
    /// for the same message.
    ///
    /// If it starts a `COPY ... FROM STDIN`, PostgreSQL ignores the `Sync` messages sent from now
    /// until the copy ends, see [`Self::copy_data_sent`].
    pub fn execution_sent(&self) {
        self.execution_generation
            .store(self.sync_generation(), Ordering::Release);
    }

    /// Record that PostgreSQL started a `COPY ... FROM STDIN`, before its `CopyInResponse` is
    /// forwarded to the client.
    pub fn copy_in_started(&self) {
        self.copy_in_response.store(true, Ordering::Release);
    }

    /// Record that a `CopyData`, or with `end` a `CopyDone` or `CopyFail`, is about to be
    /// forwarded.
    ///
    /// PostgreSQL ignores `Sync` during copy-in, so the first copy message takes back the Syncs
    /// counted since the `Execute` that started the copy, and no Sync counts until it ends.
    ///
    /// PostgreSQL also ignores copy messages when no copy is running. A client only starts
    /// sending copy data once it has the `CopyInResponse`, so a copy message without one is
    /// not counted, and cannot stop later Syncs from counting.
    pub fn copy_data_sent(&self, end: bool) {
        if !self.copy_in.load(Ordering::Acquire) {
            if !self.copy_in_response.swap(false, Ordering::AcqRel) {
                return;
            }
            let started = self.execution_generation.load(Ordering::Acquire);
            self.sync_generation.store(started, Ordering::Release);
        }
        self.copy_in.store(!end, Ordering::Release);
    }

    /// The generation of the batch PostgreSQL is currently responding to.
    fn ready_generation(&self) -> u64 {
        self.ready_generation.load(Ordering::Acquire)
    }

    ///
    /// Handle a `ReadyForQuery` from PostgreSQL.
    ///
    /// PostgreSQL has now answered, or skipped after an error, every message sent before the
    /// matching `Sync`/`Query`, so any Describe, Execute, session metrics or pending
    /// deallocation still queued for that batch will never be completed and is discarded.
    /// Without this, prepare-only statements, pipelined batches that fail part-way and
    /// statements rejected by Proxy leave state behind for the life of the connection.
    ///
    /// Portals outlive a `Sync` while a transaction is open, so they are only discarded once
    /// the connection is idle (`transaction_status` `I`).
    ///
    /// The `ReadyForQuery` sent after startup answers the `StartupMessage`, so a batch the client
    /// pipelined before it keeps its state. A `ReadyForQuery` with no outstanding request is
    /// ignored.
    ///
    pub fn ready_for_query(&mut self, transaction_status: u8) {
        let sent = self.sync_generation();
        let answered = self.ready_generation();
        if answered >= sent {
            return;
        }
        let answered = answered + 1;
        self.ready_generation.store(answered, Ordering::Release);

        debug!(target: CONTEXT, client_id = self.client_id, msg = "ReadyForQuery", generation = answered);

        let _ = self
            .describe
            .write()
            .map(|mut queue| queue.discard_before(answered));
        let _ = self
            .execute
            .write()
            .map(|mut queue| queue.discard_before(answered));
        // A statement prepared in an earlier batch is usually executed in a later one, so its
        // session must survive until then. Retention stays bounded by the live statements.
        let live_sessions: HashSet<SessionId> = self
            .statement_sessions
            .read()
            .map(|sessions| sessions.values().copied().collect())
            .unwrap_or_default();
        let _ = self.session_metrics.write().map(|mut queue| {
            queue.retain_from(answered, |session| live_sessions.contains(&session.id()))
        });
        let _ = self.pending_deallocations.write().map(|mut pending| {
            while pending.front().is_some_and(|p| p.generation < answered) {
                pending.pop_front();
            }
        });
        let _ = self.refusals.write().map(|mut refusals| {
            while refusals
                .front()
                .is_some_and(|refusal| refusal.generation < answered)
            {
                refusals.pop_front();
            }
        });
        if let Ok(mut settings) = self.settings.lock() {
            while settings
                .pending
                .front()
                .is_some_and(|p| p.generation < answered)
            {
                settings.pending.pop_front();
            }
            settings.confirmed_mut().ready_for_query(transaction_status);
        }

        if transaction_status == b'I' {
            self.discard_portals_before(answered);
        }
        self.batch_answered.notify_waiters();
    }

    /// Wait until PostgreSQL has answered every batch sent so far that may change the settings.
    ///
    /// Within a batch, a statement runs only if the statements before it succeed, so it can be
    /// mapped under the settings projected from them. A later batch runs whether or not they
    /// succeed, and after a `SET LOCAL` in the batch has ended, so it is mapped under the
    /// settings PostgreSQL confirms.
    pub async fn settings_barrier(&self) {
        loop {
            let answered = self.batch_answered.notified();
            tokio::pin!(answered);
            answered.as_mut().enable();
            if !self.settings_in_flight() {
                return;
            }
            debug!(target: CONTEXT, client_id = self.client_id, msg = "Waiting for settings");
            answered.await;
        }
    }

    /// A batch that may change the settings has been sent and not answered yet
    fn settings_in_flight(&self) -> bool {
        // A poisoned lock still waits, so Proxy fails closed
        let barrier = self
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .barrier;
        barrier.is_some_and(|generation| {
            generation < self.sync_generation() && generation >= self.ready_generation()
        })
    }

    fn discard_portals_before(&mut self, generation: u64) {
        let _ = self.portals.write().map(|mut portals| {
            portals.retain(|_, queue| {
                queue.discard_before(generation);
                !queue.is_empty()
            });
        });
        // By generation, not by `portals`: with mapping disabled a Bind adds no portal, and the
        // portal it bound for the next batch must keep its effects
        let _ = self
            .portal_deallocations
            .write()
            .map(|mut guarded| guarded.retain(|_, (bound, _)| *bound >= generation));
        let _ = self
            .portal_settings
            .write()
            .map(|mut guarded| guarded.retain(|_, (bound, _)| *bound >= generation));
        let _ = self
            .bound_settings
            .write()
            .map(|mut guarded| guarded.retain(|_, (bound, _)| *bound >= generation));
    }

    /// Queue SQL deallocations from a simple `Query` that is about to be forwarded to PostgreSQL.
    pub fn add_deallocations(&mut self, deallocations: Vec<Deallocation>) {
        if deallocations.is_empty() {
            return;
        }
        let generation = self.sync_generation();
        let parsed_before = self.parse_sequence.load(Ordering::Acquire);
        let _ = self.pending_deallocations.write().map(|mut pending| {
            pending.extend(
                deallocations
                    .into_iter()
                    .map(|deallocation| PendingDeallocation {
                        generation,
                        parsed_before,
                        deallocation,
                    }),
            );
        });
    }

    ///
    /// Handle a `CommandComplete` from PostgreSQL for a `DEALLOCATE` or `DISCARD ALL`.
    ///
    /// Applies the oldest queued deallocation for the batch being answered, removing the
    /// statements PostgreSQL has just removed. Statements parsed after the deallocation was sent
    /// keep their state even when they reuse a deallocated name.
    ///
    pub fn complete_deallocation(&mut self) {
        let answering = self.ready_generation();
        let pending = self
            .pending_deallocations
            .write()
            .ok()
            .and_then(|mut pending| {
                if pending.front().is_some_and(|p| p.generation == answering) {
                    pending.pop_front()
                } else {
                    None
                }
            });

        let Some(PendingDeallocation {
            parsed_before,
            deallocation,
            ..
        }) = pending
        else {
            return;
        };

        debug!(target: CONTEXT, client_id = self.client_id, msg = "Deallocate", ?deallocation);

        let names = match &deallocation {
            Deallocation::Statement(name) => vec![name.clone()],
            Deallocation::AllStatements => {
                let mut names = self
                    .statement_sequences
                    .read()
                    .map(|s| s.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                if let Ok(statements) = self.statements.read() {
                    names.extend(statements.keys().cloned());
                }
                if let Ok(statements) = self.statement_deallocations.read() {
                    names.extend(statements.keys().cloned());
                }
                // The unnamed statement is not removed by DEALLOCATE ALL or DISCARD ALL
                names.retain(|name| !name.is_unnamed());
                let mut seen = HashSet::new();
                names.retain(|name| seen.insert(name.clone()));
                names
            }
        };

        for name in names {
            // A statement with no recorded Parse is kept: leaking it is safe, while removing a
            // statement PostgreSQL still has would let its next Bind through unencrypted.
            let parsed_in_time = self
                .statement_sequences
                .read()
                .ok()
                .and_then(|s| s.get(&name).copied())
                .is_some_and(|sequence| sequence <= parsed_before);
            if parsed_in_time {
                self.close_statement(&name);
            }
        }
    }

    /// Queue the settings commands of a simple `Query`, one for each statement, that is about to
    /// be forwarded to PostgreSQL.
    ///
    /// A `Query` with no settings command queues nothing, as PostgreSQL's answers to it change
    /// no setting.
    pub fn add_settings_commands(&mut self, commands: Vec<SettingsCommand>) {
        if commands
            .iter()
            .all(|command| matches!(command, SettingsCommand::Other))
        {
            return;
        }
        self.queue_settings_commands(commands);
    }

    fn queue_settings_commands(&mut self, commands: Vec<SettingsCommand>) {
        let generation = self.sync_generation();
        // A poisoned lock still queues the entries, so the batch's responses stay in step
        let mut settings = self
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for command in commands {
            settings.push(PendingSettings {
                generation,
                command,
            });
        }
    }

    ///
    /// Handle a `CommandComplete`, `EmptyQueryResponse` or `PortalSuspended` from PostgreSQL,
    /// with `tag` the `CommandComplete` tag.
    ///
    /// Applies the oldest queued settings command for the batch being answered, so a setting
    /// changes only once PostgreSQL has run the command, in the order PostgreSQL ran it. A
    /// `COMMIT` confirmed with `ROLLBACK` rolls back.
    ///
    pub fn settings_completed(&mut self, tag: &str) {
        let answering = self.ready_generation();
        let Ok(mut settings) = self.settings.lock() else {
            return;
        };
        if !settings
            .pending
            .front()
            .is_some_and(|p| p.generation == answering)
        {
            return;
        }
        let Some(PendingSettings { command, .. }) = settings.pop_front() else {
            return;
        };
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Settings command", ?command, tag);
        settings.confirmed.command_succeeded(&command, tag);
    }

    ///
    /// Handle an `ErrorResponse` from PostgreSQL.
    ///
    /// PostgreSQL skips the rest of the batch being answered, so its queued settings commands
    /// never run, and fails the current transaction, which undoes its settings changes once it
    /// is rolled back.
    ///
    pub fn settings_failed(&mut self) {
        let answering = self.ready_generation();
        let Ok(mut settings) = self.settings.lock() else {
            return;
        };
        while settings
            .pending
            .front()
            .is_some_and(|p| p.generation == answering)
        {
            settings.pending.pop_front();
        }
        settings.confirmed_mut().execution_failed();
    }

    /// The settings statements sent now run with: the confirmed settings, with every command
    /// PostgreSQL has not answered yet assumed to succeed.
    ///
    /// Fails closed, with mapping enabled and no keyset, if the settings are unavailable.
    pub fn projected_settings(&self) -> ConnectionSettings {
        self.settings
            .lock()
            .map(|mut settings| settings.projected())
            .unwrap_or_default()
    }

    pub fn get_portal(&self, name: &Name) -> Option<Arc<Portal>> {
        debug!(target: CONTEXT, client_id = self.client_id, src = "Get Portal", portal = ?name);
        let portals = self.portals.read().ok()?;

        let queue = portals.get(name)?;
        queue.next().cloned()
    }

    pub fn get_portal_statement(&self, name: &Name) -> Option<Arc<Statement>> {
        let portals = self.portals.read().ok()?;
        let queue = portals.get(name)?;
        let portal = queue.next()?;

        debug!(target: CONTEXT, client_id = self.client_id, portal = ?portal);

        match portal.as_ref() {
            Portal::Encrypted { statement, .. } => Some(statement.clone()),
            Portal::Passthrough { .. } => None,
        }
    }

    pub fn get_portal_session_id(&self, name: &Name) -> Option<SessionId> {
        let portals = self.portals.read().ok()?;
        let queue = portals.get(name)?;
        let portal = queue.next()?;
        portal.session_id()
    }

    pub fn get_statement_for_row_decription(&self) -> Option<Arc<Statement>> {
        if let Some(statement) = self.get_statement_from_describe() {
            return Some(statement.clone());
        }

        if let Some(Portal::Encrypted { statement, .. }) = self.get_portal_from_execute().as_deref()
        {
            return Some(statement.clone());
        };

        None
    }

    pub fn get_statement_from_describe(&self) -> Option<Arc<Statement>> {
        let queue = self.describe.read().ok()?;
        let describe = queue.next()?;

        debug!(target: CONTEXT, client_id = self.client_id, msg = "Get Statement", describe = ?describe);

        match describe {
            Describe {
                ref name,
                target: Target::Portal,
            } => self.get_portal_statement(name),
            Describe {
                ref name,
                target: Target::Statement,
            } => self.get_statement(name),
        }
    }

    pub fn get_portal_from_execute(&self) -> Option<Arc<Portal>> {
        let queue = self.execute.read().ok()?;
        let execute_context = queue.next()?;
        let name = &execute_context.name;
        self.get_portal(name)
    }

    pub fn get_execute(&self) -> Option<ExecuteContext> {
        let queue = self.execute.read().ok()?;
        let execute_context = queue.next()?;
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Get Execute", execute = ?execute_context);
        Some(execute_context.to_owned())
    }

    pub fn get_session_metrics(&self) -> Option<SessionMetricsContext> {
        let queue = self.session_metrics.read().ok()?;
        let session_context = queue.next()?;
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Get Session Metrics", session_metrics = ?session_context);
        Some(session_context.to_owned())
    }

    pub fn set_schema_changed(&self) {
        debug!(target: CONTEXT,
            client_id = self.client_id,
            msg = "Schema changed"
        );
        let _ = self.schema_changed.write().map(|mut guard| *guard = true);
    }

    pub fn schema_changed(&self) -> bool {
        self.schema_changed.read().ok().is_some_and(|s| *s)
    }

    pub fn get_table_resolver(&self) -> Arc<TableResolver> {
        self.table_resolver.clone()
    }

    /// Whether `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true` is in effect, as PostgreSQL has
    /// confirmed it. Statements are mapped with [`Context::projected_settings`] instead.
    #[cfg(test)]
    pub fn unsafe_disable_mapping(&self) -> bool {
        self.settings
            .lock()
            .is_ok_and(|settings| settings.confirmed.unsafe_disable_mapping())
    }

    /// The keyset selected with `SET CIPHERSTASH.KEYSET_ID` or `KEYSET_NAME`, as PostgreSQL has
    /// confirmed it
    pub fn keyset_identifier(&self) -> Option<KeysetIdentifier> {
        self.settings
            .lock()
            .ok()
            .and_then(|settings| settings.confirmed.keyset())
    }

    // Service delegation methods
    /// Encrypt with `keyset_id`, the keyset the statement being mapped will run with, see
    /// [`Self::projected_settings`]
    pub async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<crate::EqlCiphertext>>, Error> {
        let expected = plaintexts.len();

        let encrypted = self
            .encryption
            .encrypt(keyset_id, plaintexts, columns)
            .await?;

        // Values are paired with parameters by position, so a short result must never be used
        if encrypted.len() != expected {
            return Err(EncryptError::EncryptionResultCountMismatch {
                expected,
                actual: encrypted.len(),
            }
            .into());
        }
        Ok(encrypted)
    }

    pub async fn decrypt(
        &self,
        ciphertexts: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
        let keyset_id = self.keyset_identifier();
        self.encryption.decrypt(keyset_id, ciphertexts).await
    }

    pub async fn reload_schema(&self) {
        let (responder, receiver) = oneshot::channel();
        match self
            .reload_sender
            .send(ReloadCommand::DatabaseSchema(responder))
        {
            Ok(_) => (),
            Err(err) => {
                error!(
                    msg = "Database schema could not be reloaded",
                    error = err.to_string()
                );
            }
        }

        debug!(target: CONTEXT, msg = "Waiting for schema reload");
        let response = receiver.await;
        debug!(target: CONTEXT, msg = "Database schema reloaded", ?response);
    }

    /// Reload schema if it has changed since last check.
    pub async fn reload_schema_if_changed(&self) {
        if self.schema_changed() {
            self.reload_schema().await;
        }
    }

    pub fn is_passthrough(&self) -> bool {
        self.encrypt_config.is_empty() || self.config.mapping_disabled()
    }

    // Column processing delegation methods
    pub fn get_projection_columns(
        &self,
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        self.column_mapper.get_projection_columns(typed_statement)
    }

    pub fn get_param_columns(
        &self,
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        self.column_mapper.get_param_columns(typed_statement)
    }

    pub fn get_literal_columns(
        &self,
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        self.column_mapper.get_literal_columns(typed_statement)
    }

    // Direct config access methods
    pub fn connection_timeout(&self) -> Option<std::time::Duration> {
        self.config.database.connection_timeout()
    }

    pub fn mapping_disabled(&self) -> bool {
        self.config.mapping_disabled()
    }

    pub fn mapping_errors_enabled(&self) -> bool {
        self.config.mapping_errors_enabled()
    }

    pub fn slow_db_response_min_duration(&self) -> std::time::Duration {
        self.config.slow_db_response_min_duration()
    }

    pub fn prometheus_enabled(&self) -> bool {
        self.config.prometheus_enabled()
    }

    pub fn default_keyset_id(&self) -> Option<KeysetIdentifier> {
        self.config
            .encrypt
            .default_keyset_id
            .map(|uuid| KeysetIdentifier(IdentifiedBy::Uuid(uuid)))
    }

    // Additional config access methods for handler
    pub fn database_socket_address(&self) -> String {
        self.config.database.to_socket_address()
    }

    pub fn database_username(&self) -> &str {
        &self.config.database.username
    }

    pub fn database_password(&self) -> String {
        self.config.database.password()
    }

    pub fn tls_config(&self) -> &Option<crate::config::TlsConfig> {
        &self.config.tls
    }

    pub fn use_tls(&self) -> bool {
        self.config.tls.is_some()
    }

    pub fn require_tls(&self) -> bool {
        self.config.server.require_tls
    }

    pub fn use_structured_logging(&self) -> bool {
        self.config.use_structured_logging()
    }

    pub fn database_tls_disabled(&self) -> bool {
        self.config.database_tls_disabled()
    }

    pub fn config(&self) -> &crate::config::TandemConfig {
        &self.config
    }

    fn with_session_metrics_mut<F>(&mut self, session_id: SessionId, f: F)
    where
        F: FnOnce(&mut SessionMetricsContext),
    {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue
                .queue
                .iter_mut()
                .find(|session| session.id() == session_id)
            {
                f(session);
            }
        }
    }

    pub fn latest_session_id(&self) -> Option<SessionId> {
        let queue = self.session_metrics.read().ok()?;
        queue.queue.back().map(|session| session.id())
    }

    /// Record parse phase duration for the session (first write wins)
    pub fn record_parse_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.record_parse(duration);
        });
    }

    /// Add encrypt phase duration for the session (accumulate)
    pub fn add_encrypt_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.add_encrypt(duration);
        });
    }

    /// Record server write phase duration
    pub fn record_server_write_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.record_server_write(duration);
        });
    }

    /// Add server write phase duration (accumulate)
    pub fn add_server_write_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.add_server_write(duration);
        });
    }

    /// Record server wait phase duration (time to first response byte)
    pub fn record_server_wait_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.record_server_wait(duration);
        });
    }

    /// Record server response phase duration
    pub fn record_server_response_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.record_server_response(duration);
        });
    }

    /// Add server response phase duration (accumulate)
    pub fn add_server_response_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.add_server_response(duration);
        });
    }

    /// Record client write phase duration
    pub fn record_client_write_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.record_client_write(duration);
        });
    }

    /// Add client write phase duration (accumulate)
    pub fn add_client_write_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.add_client_write(duration);
        });
    }

    /// Add decrypt phase duration (accumulate)
    pub fn add_decrypt_duration(&mut self, session_id: SessionId, duration: Duration) {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.add_decrypt(duration);
        });
    }

    /// Update statement metadata for a session
    pub fn update_statement_metadata<F>(&mut self, session_id: SessionId, f: F)
    where
        F: FnOnce(&mut StatementMetadata),
    {
        self.with_session_metrics_mut(session_id, |session| {
            f(&mut session.metadata);
        });
    }

    /// Update statement metadata if session ID is present, no-op otherwise.
    pub fn with_session<F>(&mut self, session_id: Option<SessionId>, f: F)
    where
        F: FnOnce(&mut SessionMetricsContext),
    {
        if let Some(sid) = session_id {
            self.with_session_metrics_mut(sid, f);
        }
    }

    /// Record server wait for first response; otherwise accumulate response time for the current execute
    pub fn record_execute_server_timing(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.execute.write() {
            if let Some(execute) = queue.current_mut() {
                execute.record_server_wait_or_add_response(duration);
            }
        }
    }

    /// Add decrypt phase duration for the current execute session (if any)
    pub fn add_decrypt_duration_for_execute(&mut self, duration: Duration) {
        let session_id = self.get_execute().and_then(|execute| execute.session_id());
        if let Some(session_id) = session_id {
            self.add_decrypt_duration(session_id, duration);
        }
    }

    /// Add client write duration for the current execute session (if any)
    pub fn add_client_write_duration_for_execute(&mut self, duration: Duration) {
        let session_id = self.get_execute().and_then(|execute| execute.session_id());
        if let Some(session_id) = session_id {
            self.add_client_write_duration(session_id, duration);
        }
    }
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Queue {
            queue: VecDeque::new(),
            generations: VecDeque::new(),
        }
    }

    pub fn complete(&mut self) {
        let _ = self.queue.pop_front();
        let _ = self.generations.pop_front();
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Remove entries created before `generation`
    pub fn discard_before(&mut self, generation: u64) {
        while self.generations.front().is_some_and(|g| *g < generation) {
            self.complete();
        }
    }

    /// Discards items queued before `generation` unless `keep` returns true for them.
    pub fn retain_from(&mut self, generation: u64, keep: impl Fn(&T) -> bool) {
        let items = std::mem::take(&mut self.queue);
        let generations = std::mem::take(&mut self.generations);
        for (item, g) in items.into_iter().zip(generations) {
            if g >= generation || keep(&item) {
                self.queue.push_back(item);
                self.generations.push_back(g);
            }
        }
    }

    pub fn next(&self) -> Option<&T> {
        self.queue.front()
    }

    /// Generation of the first item in the queue
    pub fn next_generation(&self) -> Option<u64> {
        self.generations.front().copied()
    }

    pub fn add(&mut self, item: T, generation: u64) {
        self.queue.push_back(item);
        self.generations.push_back(generation);
    }

    /// Get mutable reference to the current (first) item in the queue
    pub fn current_mut(&mut self) -> Option<&mut T> {
        self.queue.front_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, Deallocation, Describe, KeysetIdentifier, Portal, StateSizes, Statement};
    use crate::{
        config::LogConfig,
        error::Error,
        log,
        postgresql::{
            messages::{Name, Target},
            Column,
        },
        proxy::{EncryptConfig, EncryptionService},
        TandemConfig,
    };
    use eql_mapper::Schema;
    use sqltk::parser::{dialect::PostgreSqlDialect, parser::Parser};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    struct TestService {}

    #[async_trait::async_trait]
    impl EncryptionService for TestService {
        async fn encrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
            _columns: &[Option<Column>],
        ) -> Result<Vec<Option<crate::EqlCiphertext>>, Error> {
            Ok(vec![])
        }

        async fn decrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(vec![])
        }
    }

    fn create_context() -> Context<TestService> {
        let client_id = 1;
        let config = Arc::new(TandemConfig::for_testing());
        let encrypt_config = Arc::new(EncryptConfig::default());
        let schema = Arc::new(Schema::new("public"));

        let (reload_sender, _reload_receiver) = mpsc::unbounded_channel();

        let service = TestService {};

        Context::new(
            client_id,
            config,
            encrypt_config,
            schema,
            service,
            reload_sender,
        )
    }

    fn statement() -> Statement {
        Statement {
            param_columns: vec![],
            projection_columns: vec![],
            literal_columns: vec![],
            postgres_param_types: vec![],
        }
    }

    fn portal(statement: &Arc<Statement>) -> Portal {
        Portal::encrypted_with_format_codes(statement.clone(), vec![], None)
    }

    fn get_statement(portal: Arc<Portal>) -> Arc<Statement> {
        match portal.as_ref() {
            Portal::Encrypted { statement, .. } => statement.clone(),
            _ => {
                panic!("Expected Encrypted Portal");
            }
        }
    }

    #[test]
    pub fn get_statement_from_describe() {
        log::init(LogConfig::default());

        let mut context = create_context();

        let name = Name::from("name");

        context.add_statement(name.clone(), statement());

        let statement = context.get_statement(&name).unwrap();

        let describe = Describe {
            name,
            target: Target::Statement,
        };
        context.set_describe(describe);

        let s = context.get_statement_from_describe().unwrap();

        assert_eq!(s, statement)
    }

    #[test]
    pub fn execution_flow() {
        log::init(LogConfig::default());

        let mut context = create_context();
        context.ready_for_query(b'I');

        let statement_name = Name::from("statement");
        let portal_name = Name::from("portal");

        // Add statement to context
        context.add_statement(statement_name.clone(), statement());

        // Get statement from context
        let statement = context.get_statement(&statement_name).unwrap();

        // Add portal pointing to statement to context
        context.add_portal(portal_name.clone(), portal(&statement));

        // Add statement name to execute context
        context.set_execute(portal_name.clone(), None);

        // Portal statement should be the right statement
        let portal = context.get_portal_from_execute().unwrap();

        let statement = get_statement(portal);
        assert_eq!(statement, statement);

        // Complete the execution
        context.complete_execution();

        // Should be no portal for execute context
        let portal = context.get_portal_from_execute();
        assert!(portal.is_none());

        // Unamed portal is closed on complete
        let portal = context.get_portal(&portal_name);
        assert!(portal.is_some());
    }

    /// Unit test for the queue-draining primitives.
    ///
    /// `complete_execution()` / `finish_session()` are the only drains for the
    /// per-connection `execute` / `session_metrics` queues. This asserts that
    /// calling them after enqueuing a session + execute leaves both queues
    /// empty, across many iterations.
    ///
    /// Note: this exercises the primitives directly — it does *not* drive the
    /// backend passthrough path that actually caused BUG-300 (that early
    /// returned without calling these). The backend-level regression test for
    /// BUG-300 lives in `backend.rs`
    /// (`passthrough_drains_queues_on_execute_terminating_message`).
    #[test]
    pub fn complete_execution_and_finish_session_drain_queues() {
        log::init(LogConfig::default());

        let mut context = create_context();
        context.ready_for_query(b'I');

        for _ in 0..1000 {
            // Frontend: a session + execute are enqueued for every statement.
            let session_id = context.start_session();
            context.set_execute(Name::unnamed(), Some(session_id));

            // Drain primitives, normally called by the backend on an
            // execute-terminating message (CommandComplete / ErrorResponse / …).
            context.complete_execution();
            context.finish_session();
        }

        assert_eq!(context.execute_queue_len(), 0);
        assert_eq!(context.session_metrics_queue_len(), 0);
    }

    #[test]
    pub fn add_and_close_portals() {
        log::init(LogConfig::default());

        let mut context = create_context();

        // Create multiple statements
        let statement_name_1 = Name::from("statement_1");
        let statement_name_2 = Name::from("statement_2");

        // Add statements to context
        context.add_statement(statement_name_1.clone(), statement());
        context.add_statement(statement_name_2.clone(), statement());

        // Replicate pipelined execution
        // Add multiple portals with the same name
        // Pointing to different statements
        let portal_name = Name::from("portal");

        let statement_1 = context.get_statement(&statement_name_1).unwrap();
        context.add_portal(portal_name.clone(), portal(&statement_1));

        let statement_2 = context.get_statement(&statement_name_2).unwrap();
        context.add_portal(portal_name.clone(), portal(&statement_2));

        // Execute both portals
        context.set_execute(portal_name.clone(), None);
        context.set_execute(portal_name.clone(), None);

        // Portal should point to first statement
        let portal = context.get_portal_from_execute().unwrap();
        let statement = get_statement(portal);
        assert_eq!(statement, statement);

        let portal = context.get_portal_from_execute().unwrap();
        let statement = get_statement(portal);
        assert_eq!(statement_1, statement);

        // Complete execution
        context.complete_execution();

        // Portal should point to second statement
        let portal = context.get_portal_from_execute().unwrap();

        let statement = get_statement(portal);
        assert_eq!(statement_1, statement);
    }

    #[test]
    pub fn pipeline_execution() {
        log::init(LogConfig::default());

        let mut context = create_context();

        let statement_name_1 = Name::from("statement_1");
        let portal_name_1 = Name::unnamed();

        let statement_name_2 = Name::from("statement_2");
        let portal_name_2 = Name::unnamed();

        let statement_name_3 = Name::from("statement_3");
        let portal_name_3 = Name::from("portal_3");

        // Add statement to context
        context.add_statement(statement_name_1.clone(), statement());
        context.add_statement(statement_name_2.clone(), statement());
        context.add_statement(statement_name_3.clone(), statement());

        // Create portals for each statement
        let statement_1 = context.get_statement(&statement_name_1).unwrap();
        context.add_portal(portal_name_1.clone(), portal(&statement_1));

        let statement_2 = context.get_statement(&statement_name_2).unwrap();
        context.add_portal(portal_name_2.clone(), portal(&statement_2));

        let statement_3 = context.get_statement(&statement_name_3).unwrap();
        context.add_portal(portal_name_3.clone(), portal(&statement_3));

        // Add portals to execute context
        context.set_execute(portal_name_1.clone(), None);
        context.set_execute(portal_name_2.clone(), None);
        context.set_execute(portal_name_3.clone(), None);

        // Multiple calls return the portal for the first Execution context
        let portal = context.get_portal_from_execute().unwrap();
        let statement = get_statement(portal);
        assert_eq!(statement_1, statement);

        let portal = context.get_portal_from_execute().unwrap();
        let statement = get_statement(portal);
        assert_eq!(statement_1, statement);

        // Complete the execution of the first portal
        context.complete_execution();

        // Returns the next portal
        let portal = context.get_portal_from_execute().unwrap();
        let statement = get_statement(portal);
        assert_eq!(statement_2, statement);

        // Complete the execution
        context.complete_execution();

        // Returns the next portal
        let portal = context.get_portal_from_execute().unwrap();
        let statement = get_statement(portal);
        assert_eq!(statement_3, statement);
    }

    fn parse_statement(sql: &str) -> sqltk::parser::ast::Statement {
        let statements = Parser::new(&PostgreSqlDialect {})
            .try_with_sql(sql)
            .unwrap()
            .parse_statements()
            .unwrap();

        statements.first().unwrap().clone()
    }

    // Connection state lifecycle
    //
    // Each helper makes the Context calls the frontend and backend make for one protocol message,
    // so a test reads as the message flow between client, Proxy and PostgreSQL.

    const ITERATIONS: usize = 1000;

    fn parse(context: &mut Context<TestService>, name: &Name) {
        let session_id = context.start_session();
        context.set_statement_session(name.clone(), session_id);
        context.add_statement(name.clone(), statement());
    }

    fn bind(context: &mut Context<TestService>, statement_name: &Name, portal_name: &Name) {
        let statement = context.get_statement(statement_name).unwrap();
        context.add_portal(portal_name.clone(), portal(&statement));
    }

    fn describe(context: &mut Context<TestService>, name: &Name, target: Target) {
        context.set_describe(Describe {
            name: name.clone(),
            target,
        });
    }

    /// Frontend forwards a simple Query
    fn simple_query(context: &mut Context<TestService>, deallocations: Vec<Deallocation>) {
        let session_id = context.start_session();
        context.add_portal(Name::unnamed(), Portal::passthrough(Some(session_id)));
        context.set_execute(Name::unnamed(), Some(session_id));
        context.add_deallocations(deallocations);
        context.sync_sent();
    }

    /// Backend receives CommandComplete, EmptyQueryResponse, PortalSuspended or ErrorResponse
    fn execution_finished(context: &mut Context<TestService>) {
        context.complete_execution();
        context.finish_session();
    }

    fn empty() -> StateSizes {
        StateSizes::default()
    }

    #[test]
    pub fn startup_ready_for_query_is_ignored() {
        let mut context = create_context();
        context.ready_for_query(b'I');

        let name = Name::unnamed();
        parse(&mut context, &name);
        bind(&mut context, &name, &name);
        context.set_execute_for_portal(name.clone());

        // An unsolicited ReadyForQuery must not discard the batch that has not been sent yet
        context.ready_for_query(b'I');
        assert_eq!(context.state_sizes().execute, 1);
        assert_eq!(context.state_sizes().session_metrics, 1);
    }

    #[test]
    pub fn batch_pipelined_before_the_startup_ready_for_query_keeps_its_state() {
        let mut context = create_context();

        // The client sends a whole batch before PostgreSQL finishes starting up
        let name = Name::unnamed();
        parse(&mut context, &name);
        bind(&mut context, &name, &name);
        describe(&mut context, &name, Target::Portal);
        context.set_execute_for_portal(name.clone());
        context.sync_sent();

        // The startup ReadyForQuery answers the StartupMessage, not the batch
        context.ready_for_query(b'I');

        let sizes = context.state_sizes();
        assert_eq!(sizes.describe, 1);
        assert_eq!(sizes.execute, 1);
        assert_eq!(sizes.portals, 1);
        assert!(context.get_portal_statement(&name).is_some());
    }

    #[test]
    pub fn error_in_a_batch_without_execute_keeps_the_next_batch_execute() {
        let mut context = create_context();
        context.ready_for_query(b'I');

        // Batch 1: Parse(bad), Sync. PostgreSQL rejects the Parse.
        let session_id = context.start_session();
        context.set_statement_session(Name::from("bad"), session_id);
        context.sync_sent();

        // Batch 2, pipelined: Parse, Bind, Execute, Sync
        let name = Name::unnamed();
        parse(&mut context, &name);
        bind(&mut context, &name, &name);
        context.set_execute_for_portal(name.clone());
        context.sync_sent();

        // Backend: ErrorResponse and ReadyForQuery for batch 1
        execution_finished(&mut context);
        context.ready_for_query(b'I');

        // Batch 2's rows still find their portal
        assert_eq!(context.state_sizes().execute, 1);
        assert!(context
            .get_execute()
            .is_some_and(|execute| execute.name == name));
    }

    #[test]
    pub fn syncs_ignored_during_copy_in_keep_later_batches_in_step() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::from("s1");
        parse(&mut context, &name);
        context.sync_sent();
        context.ready_for_query(b'I');

        // Execute(COPY ... FROM STDIN), Sync, then CopyData and CopyDone once PostgreSQL sends
        // CopyInResponse, then Sync
        let unnamed = Name::unnamed();
        parse(&mut context, &unnamed);
        bind(&mut context, &unnamed, &unnamed);
        context.set_execute_for_portal(unnamed.clone());
        context.execution_sent();
        context.sync_sent();
        context.copy_in_started();
        context.copy_data_sent(false);
        context.copy_data_sent(true);
        context.sync_sent();

        // PostgreSQL ignored the first Sync, so one CommandComplete and one ReadyForQuery
        execution_finished(&mut context);
        context.ready_for_query(b'I');

        // DEALLOCATE s1 in the next batch still releases the statement
        simple_query(&mut context, vec![Deallocation::Statement(name.clone())]);
        context.execution_sent();
        execution_finished(&mut context);
        context.complete_deallocation();
        assert_eq!(context.state_sizes().execute, 0);
        context.ready_for_query(b'I');

        assert!(context.get_statement(&name).is_none());
    }

    #[test]
    pub fn copy_data_outside_a_copy_keeps_batches_in_step() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let (a1, a2) = (Name::from("a1"), Name::from("a2"));
        parse(&mut context, &a1);
        parse(&mut context, &a2);
        context.sync_sent();
        context.ready_for_query(b'I');

        // A CopyData with no COPY running, which PostgreSQL ignores
        context.copy_data_sent(false);

        // "SELECT 1/0; DEALLOCATE a1" fails before PostgreSQL reaches the DEALLOCATE
        simple_query(&mut context, vec![Deallocation::Statement(a1.clone())]);
        context.execution_sent();
        execution_finished(&mut context);
        context.ready_for_query(b'I');

        // "DEALLOCATE a2" succeeds
        simple_query(&mut context, vec![Deallocation::Statement(a2.clone())]);
        context.execution_sent();
        execution_finished(&mut context);
        context.complete_deallocation();
        context.ready_for_query(b'I');

        // PostgreSQL still has a1, so its next Bind must still be encrypted
        assert!(context.get_statement(&a1).is_some());
        assert!(context.get_statement(&a2).is_none());
    }

    #[test]
    pub fn balanced_extended_protocol_retains_nothing() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::unnamed();
        for _ in 0..ITERATIONS {
            parse(&mut context, &name);
            bind(&mut context, &name, &name);
            describe(&mut context, &name, Target::Portal);
            context.set_execute_for_portal(name.clone());
            context.sync_sent();
            context.complete_describe();
            execution_finished(&mut context);
            context.ready_for_query(b'I');
        }
        let sizes = context.state_sizes();
        assert_eq!(sizes.describe, 0);
        assert_eq!(sizes.execute, 0);
        assert_eq!(sizes.session_metrics, 0);
        assert_eq!(sizes.portals, 0);
    }

    #[test]
    pub fn prepare_without_execute_retains_only_the_live_statement_session() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::unnamed();
        for _ in 0..ITERATIONS {
            parse(&mut context, &name);
            describe(&mut context, &name, Target::Statement);
            context.sync_sent();
            context.complete_describe();
            context.ready_for_query(b'I');
        }
        // Only the session of the statement that still exists is kept for a later execution
        assert_eq!(context.state_sizes().session_metrics, 1);
        assert_eq!(context.state_sizes().describe, 0);
    }

    #[test]
    pub fn statement_prepared_in_an_earlier_batch_keeps_its_session_for_execution() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::unnamed();

        // Parse, Describe, Sync
        parse(&mut context, &name);
        let session_id = context.get_statement_session(&name).unwrap();
        describe(&mut context, &name, Target::Statement);
        context.sync_sent();
        context.complete_describe();
        context.ready_for_query(b'I');

        // Bind, Execute, Sync
        bind(&mut context, &name, &name);
        context.set_execute_for_portal(name.clone());
        context.sync_sent();
        assert_eq!(context.latest_session_id(), Some(session_id));
        execution_finished(&mut context);
        context.ready_for_query(b'I');

        let sizes = context.state_sizes();
        assert_eq!(sizes.session_metrics, 0);
        assert_eq!(sizes.execute, 0);
    }

    #[test]
    pub fn pipelined_batch_failing_part_way_retains_only_the_live_statement_session() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::unnamed();
        for _ in 0..ITERATIONS {
            for _ in 0..2 {
                parse(&mut context, &name);
                bind(&mut context, &name, &name);
                describe(&mut context, &name, Target::Portal);
                context.set_execute_for_portal(name.clone());
            }
            context.sync_sent();
            // RowDescription, then ErrorResponse for the first Execute.
            // PostgreSQL skips the second statement until Sync.
            context.complete_describe();
            execution_finished(&mut context);
            context.ready_for_query(b'I');
        }
        let sizes = context.state_sizes();
        assert_eq!(sizes.describe, 0);
        assert_eq!(sizes.execute, 0);
        // The skipped Parse's session is kept while the statement name refers to it
        assert_eq!(sizes.session_metrics, 1);
        assert_eq!(sizes.portals, 0);
    }

    #[test]
    pub fn parse_rejected_by_proxy_retains_no_session_metrics() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        for _ in 0..ITERATIONS {
            // Parse handler starts a session, then fails. Proxy answers the Sync itself.
            let _ = context.start_session();

            simple_query(&mut context, vec![]);
            execution_finished(&mut context);
            context.ready_for_query(b'I');
        }
        assert_eq!(context.state_sizes().session_metrics, 0);
    }

    #[test]
    pub fn a_later_pipelined_batch_survives_an_earlier_ready_for_query() {
        let mut context = create_context();
        context.ready_for_query(b'I');

        let first = Name::from("first");
        let second = Name::from("second");

        parse(&mut context, &first);
        bind(&mut context, &first, &Name::unnamed());
        context.set_execute_for_portal(Name::unnamed());
        context.sync_sent();

        // The client pipelines the next batch before PostgreSQL answers the first
        parse(&mut context, &second);
        bind(&mut context, &second, &Name::unnamed());
        describe(&mut context, &Name::unnamed(), Target::Portal);
        context.set_execute_for_portal(Name::unnamed());
        context.sync_sent();

        execution_finished(&mut context);
        context.ready_for_query(b'I');

        let sizes = context.state_sizes();
        assert_eq!(sizes.describe, 1);
        assert_eq!(sizes.execute, 1);
        assert_eq!(sizes.session_metrics, 1);

        let portal = context.get_portal_from_execute().unwrap();
        assert_eq!(
            get_statement(portal),
            context.get_statement(&second).unwrap()
        );
    }

    #[test]
    pub fn closed_named_portals_are_removed() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::from("S_1");
        parse(&mut context, &name);
        context.sync_sent();
        context.ready_for_query(b'T');

        for i in 0..ITERATIONS {
            let portal_name = Name::from(format!("C_{i}"));
            bind(&mut context, &name, &portal_name);
            context.set_execute_for_portal(portal_name.clone());
            context.close_portal(&portal_name);
            context.sync_sent();
            execution_finished(&mut context);
            context.ready_for_query(b'T');
        }
        assert_eq!(context.state_sizes().portals, 0);
    }

    #[test]
    pub fn named_portals_survive_until_the_transaction_ends() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let statement_name = Name::from("S_1");
        let portal_name = Name::from("C_1");

        parse(&mut context, &statement_name);
        bind(&mut context, &statement_name, &portal_name);
        context.set_execute_for_portal(portal_name.clone());
        context.sync_sent();
        // PortalSuspended
        execution_finished(&mut context);
        context.ready_for_query(b'T');
        assert!(context.get_portal(&portal_name).is_some());

        // COMMIT
        simple_query(&mut context, vec![]);
        execution_finished(&mut context);
        context.ready_for_query(b'I');
        assert!(context.get_portal(&portal_name).is_none());
    }

    #[test]
    pub fn sql_deallocate_removes_the_statement() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        for i in 0..ITERATIONS {
            let name = Name::from(format!("_pg3_{i}"));
            parse(&mut context, &name);
            bind(&mut context, &name, &Name::unnamed());
            context.set_execute_for_portal(Name::unnamed());
            context.sync_sent();
            execution_finished(&mut context);
            context.ready_for_query(b'I');

            simple_query(&mut context, vec![Deallocation::Statement(name)]);
            execution_finished(&mut context);
            context.complete_deallocation();
            context.ready_for_query(b'I');
        }
        assert_eq!(empty(), context.state_sizes());
    }

    #[test]
    pub fn sql_deallocate_all_removes_every_statement() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        for i in 0..10 {
            parse(&mut context, &Name::from(format!("a{i}")));
        }
        context.sync_sent();
        context.ready_for_query(b'I');

        simple_query(&mut context, vec![Deallocation::AllStatements]);
        execution_finished(&mut context);
        context.complete_deallocation();
        context.ready_for_query(b'I');

        assert_eq!(empty(), context.state_sizes());
    }

    #[test]
    pub fn sql_deallocate_all_keeps_the_unnamed_statement() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        parse(&mut context, &Name::unnamed());
        parse(&mut context, &Name::from("a1"));
        context.sync_sent();
        context.ready_for_query(b'I');

        simple_query(&mut context, vec![Deallocation::AllStatements]);
        execution_finished(&mut context);
        context.complete_deallocation();
        context.ready_for_query(b'I');

        assert!(context.get_statement(&Name::unnamed()).is_some());
        assert!(context.get_statement(&Name::from("a1")).is_none());
    }

    #[test]
    pub fn failed_sql_deallocate_keeps_the_statement() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::from("a1");
        parse(&mut context, &name);
        context.sync_sent();
        context.ready_for_query(b'T');

        // DEALLOCATE in an aborted transaction: ErrorResponse, no CommandComplete
        simple_query(&mut context, vec![Deallocation::Statement(name.clone())]);
        execution_finished(&mut context);
        context.ready_for_query(b'E');

        // A later DEALLOCATE of another statement must not apply the stale one
        simple_query(
            &mut context,
            vec![Deallocation::Statement(Name::from("a2"))],
        );
        execution_finished(&mut context);
        context.complete_deallocation();
        context.ready_for_query(b'I');

        assert!(context.get_statement(&name).is_some());
        assert_eq!(context.state_sizes().pending_deallocations, 0);
    }

    /// Parse, Bind, Execute and Sync `deallocation` as the unnamed statement, with
    /// PostgreSQL's responses
    fn extended_deallocation(context: &mut Context<TestService>, deallocation: Deallocation) {
        let unnamed = Name::unnamed();
        parse(context, &unnamed);
        context.set_statement_deallocation(unnamed.clone(), Some(deallocation));
        bind(context, &unnamed, &unnamed);
        context.bind_portal_deallocation(&unnamed, &unnamed);
        context.set_execute_for_portal(unnamed.clone());
        context.sync_sent();
        execution_finished(context);
        context.complete_deallocation();
        context.ready_for_query(b'I');
    }

    #[test]
    pub fn extended_protocol_sql_deallocate_removes_the_statement() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        for i in 0..ITERATIONS {
            let name = Name::from(format!("_pg3_{i}").as_str());
            parse(&mut context, &name);
            context.sync_sent();
            context.ready_for_query(b'I');
            extended_deallocation(&mut context, Deallocation::Statement(name));
        }
        // Only the unnamed statement that ran the last DEALLOCATE remains
        let sizes = context.state_sizes();
        assert_eq!(sizes.statements, 1);
        assert_eq!(sizes.statement_sessions, 1);
        assert_eq!(sizes.statement_deallocations, 1);
        assert_eq!(sizes.portal_deallocations, 0);
        assert_eq!(sizes.pending_deallocations, 0);
    }

    #[test]
    pub fn extended_protocol_sql_deallocate_all_removes_every_statement() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        for i in 0..ITERATIONS {
            parse(&mut context, &Name::from(format!("a{i}").as_str()));
        }
        context.sync_sent();
        context.ready_for_query(b'I');
        extended_deallocation(&mut context, Deallocation::AllStatements);
        let sizes = context.state_sizes();
        assert_eq!(sizes.statements, 1);
        assert_eq!(sizes.statement_sessions, 1);
        assert_eq!(sizes.pending_deallocations, 0);
    }

    /// Parse, Bind and Execute `deallocation` as the unnamed statement, without the Sync
    fn send_extended_deallocation(context: &mut Context<TestService>, deallocation: Deallocation) {
        let unnamed = Name::unnamed();
        parse(context, &unnamed);
        context.set_statement_deallocation(unnamed.clone(), Some(deallocation));
        bind(context, &unnamed, &unnamed);
        context.bind_portal_deallocation(&unnamed, &unnamed);
        context.set_execute_for_portal(unnamed.clone());
    }

    #[test]
    pub fn extended_deallocate_all_keeps_a_statement_parsed_later_in_the_same_batch() {
        let mut context = create_context();
        context.ready_for_query(b'I');

        // Parse/Bind/Execute(DEALLOCATE ALL), Parse(s_new), Sync
        send_extended_deallocation(&mut context, Deallocation::AllStatements);
        let name = Name::from("s_new");
        parse(&mut context, &name);
        context.sync_sent();

        execution_finished(&mut context);
        context.complete_deallocation();
        context.ready_for_query(b'I');

        assert!(context.get_statement(&name).is_some());
    }

    #[test]
    pub fn extended_deallocate_keeps_a_statement_reparsed_later_in_the_same_batch() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::from("s1");
        parse(&mut context, &name);
        context.sync_sent();
        context.ready_for_query(b'I');

        // Parse/Bind/Execute(DEALLOCATE s1), Parse(s1), Sync
        send_extended_deallocation(&mut context, Deallocation::Statement(name.clone()));
        parse(&mut context, &name);
        context.sync_sent();

        execution_finished(&mut context);
        context.complete_deallocation();
        context.ready_for_query(b'I');

        assert!(context.get_statement(&name).is_some());
    }

    #[test]
    pub fn sql_deallocate_keeps_a_statement_prepared_by_a_later_batch() {
        let mut context = create_context();
        context.ready_for_query(b'I');
        let name = Name::from("a1");
        parse(&mut context, &name);
        context.sync_sent();
        context.ready_for_query(b'I');

        simple_query(&mut context, vec![Deallocation::Statement(name.clone())]);
        // The client pipelines a new statement with the same name before the DEALLOCATE completes
        parse(&mut context, &name);
        context.sync_sent();

        execution_finished(&mut context);
        context.complete_deallocation();
        context.ready_for_query(b'I');

        assert!(context.get_statement(&name).is_some());
    }

    #[tokio::test]
    async fn encryption_service_returning_fewer_values_fails_closed() {
        let context = create_context();
        // TestService returns no outputs at all
        let plaintext = cipherstash_client::encryption::Plaintext::new("secret".to_owned());
        let result = context
            .encrypt(None, vec![Some(plaintext), None], &[None, None])
            .await;

        assert!(matches!(
            result,
            Err(Error::Encrypt(
                crate::error::EncryptError::EncryptionResultCountMismatch {
                    expected: 2,
                    actual: 0
                }
            ))
        ));
    }
}
