pub mod column;
mod lifecycle;
pub use lifecycle::ExecutionEnd;
pub mod phase_timing;
pub mod portal;
mod prepared_statements;
pub mod settings;
mod settings_protocol;
pub mod statement;
pub mod statement_metadata;
use self::prepared_statements::PreparedStatements;
use self::settings::ConnectionSettings;
pub use self::{phase_timing::PhaseTiming, portal::Portal, statement::Statement};
use super::{column_mapper::ColumnMapper, rewrite::Name, Column, OperationId};
use crate::{
    config::TandemConfig,
    error::{ConfigError, EncryptError, Error},
    log::{CONTEXT, SLOW_STATEMENTS},
    prometheus::{
        SLOW_STATEMENTS_TOTAL, STATEMENTS_EXECUTION_DURATION_SECONDS,
        STATEMENTS_SESSION_DURATION_SECONDS,
    },
    proxy::{
        schema::{CommittedSchemaStore, SchemaMiddleware},
        EncryptConfig, EncryptionService, ReloadCommand, ReloadSender,
    },
};
use cipherstash_client::IdentifiedBy;
use eql_mapper::{Schema, TableResolver};
use metrics::{counter, histogram};
use pg_proto::{Describe, DescribeTarget, DiagnosticResponse, TransactionStatus};
use serde_json::json;
pub use statement_metadata::StatementMetadata;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tracing::{debug, error, warn};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
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
    encryption: T,
    reload_sender: ReloadSender,
    schema_middleware: SchemaMiddleware,
    protocol_state: Arc<RwLock<ConnectionProtocolState>>,
    upstream_tls_roots: Arc<rustls::RootCertStore>,
    session_id_counter: Arc<AtomicU64>,
}

/// Context for tracking an in-flight Execute operation.
///
/// This stores only CipherStash metadata associated with pg-proto's operation;
/// pg-proto owns protocol ordering and backpressure.
#[derive(Clone, Debug)]
pub struct ExecuteContext {
    name: Name,
    portal: Option<Arc<Portal>>,
    start: Instant,
    session_id: Option<SessionId>,
    completes_at_readiness: bool,
}

impl ExecuteContext {
    fn new(
        name: Name,
        portal: Option<Arc<Portal>>,
        session_id: Option<SessionId>,
    ) -> ExecuteContext {
        ExecuteContext {
            name,
            portal,
            start: Instant::now(),
            session_id,
            completes_at_readiness: false,
        }
    }

    fn until_readiness(mut self) -> Self {
        self.completes_at_readiness = true;
        self
    }

    fn duration(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }

    fn session_id(&self) -> Option<SessionId> {
        self.session_id
    }
}

#[derive(Clone, Debug, Default)]
struct OperationContext {
    describe: Option<DescribeContext>,
    execute: Option<ExecuteContext>,
    error_response: Option<DiagnosticResponse>,
    preparation: Option<lifecycle::Preparation>,
    /// SQL deallocations in a simple Query or an executed portal, applied in order as
    /// PostgreSQL confirms each one
    deallocations: VecDeque<Deallocation>,
}

/// A prepared-statement removal requested with SQL rather than a protocol `Close`.
///
/// Applied only once PostgreSQL confirms the command with a `CommandComplete`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deallocation {
    /// `DEALLOCATE name`
    Statement(Name),
    /// `DEALLOCATE ALL` or `DISCARD ALL`
    AllStatements,
}

#[derive(Clone, Debug)]
struct DescribeContext {
    statement: Option<Arc<Statement>>,
}

#[cfg(test)]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ProtocolStateSizes {
    pub operations: usize,
    pub statement_metrics: usize,
    pub suspended_executions: usize,
    pub prepared_statements: usize,
    pub settings_statements: usize,
    pub portals: usize,
    pub portal_operations: usize,
}

#[derive(Debug)]
struct ConnectionProtocolState<K = OperationId> {
    settings: ConnectionSettings,
    settings_protocol: settings_protocol::SettingsProtocol,
    operations: HashMap<K, OperationContext>,
    statement_metrics: HashMap<SessionId, SessionMetricsContext>,
    suspended_executions: HashMap<Name, (K, ExecuteContext)>,
    prepared_statements: PreparedStatements<K>,
    portals: HashMap<Name, Arc<Portal>>,
    portal_operations: HashMap<Name, PortalOwner<K>>,
}

/// The Bind operation that created a portal, and the SQL deallocation it runs when executed
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortalOwner<K = OperationId> {
    operation: K,
    deallocation: Option<Deallocation>,
}

impl<K> Default for ConnectionProtocolState<K> {
    fn default() -> Self {
        Self {
            settings: ConnectionSettings::default(),
            settings_protocol: settings_protocol::SettingsProtocol::default(),
            operations: HashMap::new(),
            statement_metrics: HashMap::new(),
            suspended_executions: HashMap::new(),
            prepared_statements: PreparedStatements::default(),
            portals: HashMap::new(),
            portal_operations: HashMap::new(),
        }
    }
}

impl ConnectionProtocolState {
    fn install_portal(
        &mut self,
        operation: OperationId,
        name: Name,
        portal: Portal,
    ) -> Vec<SessionMetricsContext> {
        let mut candidates = Vec::new();
        candidates.extend(
            self.portals
                .remove(&name)
                .and_then(|portal| portal.session_id()),
        );
        candidates.extend(
            self.suspended_executions
                .remove(&name)
                .and_then(|(_, execute)| execute.session_id()),
        );
        self.portal_operations.insert(
            name.clone(),
            PortalOwner {
                operation,
                deallocation: None,
            },
        );
        self.portals.insert(name, Arc::new(portal));
        self.take_unreferenced_metrics(candidates)
    }

    fn restore_failed_preparation(
        &mut self,
        operation: Option<OperationId>,
    ) -> Vec<SessionMetricsContext> {
        let (restored, mut candidates) = self.settings_protocol.failed(operation);
        for restore in restored {
            if restore.statement {
                candidates.extend(self.prepared_statements.remove(&restore.name));
                if let Some(operation) = restore.metadata.statement_operation {
                    self.prepared_statements
                        .set_parse(restore.name.clone(), operation);
                }
                if let Some(deallocation) = restore.metadata.statement_deallocation {
                    self.prepared_statements
                        .set_deallocation(restore.name.clone(), deallocation);
                }
                if let Some(statement) = restore.metadata.statement {
                    self.prepared_statements
                        .set_statement(restore.name.clone(), statement);
                }
                if let Some(metrics) = restore.metadata.statement_metrics {
                    self.prepared_statements
                        .set_metrics_scope(restore.name, metrics);
                }
            } else {
                candidates.extend(
                    self.portals
                        .remove(&restore.name)
                        .and_then(|portal| portal.session_id()),
                );
                self.portal_operations.remove(&restore.name);
                candidates.extend(
                    self.suspended_executions
                        .remove(&restore.name)
                        .and_then(|(_, execute)| execute.session_id()),
                );
                if let Some(suspended) = restore.metadata.suspended {
                    self.suspended_executions
                        .insert(restore.name.clone(), suspended);
                }
                if let Some(portal) = restore.metadata.portal {
                    self.portals.insert(restore.name.clone(), portal);
                }
                if let Some(operation) = restore.metadata.portal_operation {
                    self.portal_operations.insert(restore.name, operation);
                }
            }
        }
        self.settings.execution_failed();
        self.take_unreferenced_metrics(candidates)
    }
}

impl ConnectionProtocolState {
    /// Everything Proxy keeps for the named statement, for a rollback to restore
    fn statement_snapshot(&self, name: &Name) -> settings_protocol::MetadataSnapshot {
        settings_protocol::MetadataSnapshot {
            statement: self.prepared_statements.statement(name),
            statement_metrics: self.prepared_statements.metrics_scope(name),
            statement_operation: self.prepared_statements.parse(name),
            statement_deallocation: self.prepared_statements.deallocation(name),
            ..Default::default()
        }
    }

    /// Everything Proxy keeps for the named portal, for a rollback to restore
    fn portal_snapshot(&self, name: &Name) -> settings_protocol::MetadataSnapshot {
        settings_protocol::MetadataSnapshot {
            portal: self.portals.get(name).cloned(),
            portal_operation: self.portal_operations.get(name).cloned(),
            suspended: self.suspended_executions.get(name).cloned(),
            ..Default::default()
        }
    }
}

impl<K> ConnectionProtocolState<K>
where
    K: Copy + Eq + std::hash::Hash + PartialOrd,
{
    fn remove_statement_metadata(&mut self, name: &Name) -> Option<SessionMetricsContext> {
        let session_id = self.prepared_statements.remove(name);
        if session_id.is_some_and(|id| self.metrics_referenced(id)) {
            None
        } else {
            session_id.and_then(|id| self.statement_metrics.remove(&id))
        }
    }

    fn metrics_referenced(&self, session_id: SessionId) -> bool {
        self.settings_protocol.references_metrics(session_id)
            || self.prepared_statements.references_metrics(session_id)
            || self
                .portals
                .values()
                .any(|portal| portal.session_id() == Some(session_id))
            || self.operations.values().any(|operation| {
                operation
                    .execute
                    .as_ref()
                    .is_some_and(|execute| execute.session_id() == Some(session_id))
            })
            || self
                .suspended_executions
                .values()
                .any(|(_, execute)| execute.session_id() == Some(session_id))
    }

    fn take_unreferenced_metrics(
        &mut self,
        candidates: impl IntoIterator<Item = SessionId>,
    ) -> Vec<SessionMetricsContext> {
        let mut metrics = Vec::new();
        for session_id in candidates {
            if !self.metrics_referenced(session_id) {
                metrics.extend(self.statement_metrics.remove(&session_id));
            }
        }
        metrics
    }

    fn finish_execution(
        &mut self,
        operation: &K,
        outcome: ExecutionOutcome,
    ) -> Result<ExecutionTransition, crate::error::ContextError> {
        let Some(operation_context) = self.operations.get_mut(operation) else {
            return Err(crate::error::ContextError::UnknownOperation);
        };
        if outcome == ExecutionOutcome::Completed
            && operation_context
                .execute
                .as_ref()
                .is_some_and(|execute| execute.completes_at_readiness)
        {
            return Ok(ExecutionTransition {
                execute: operation_context.execute.clone(),
                metrics: None,
                finished_metrics: None,
                replacement_error: None,
                execution_finished: false,
            });
        }
        let execute = operation_context.execute.take();
        if execute.is_none() && outcome != ExecutionOutcome::Failed {
            return Err(crate::error::ContextError::OperationWithoutExecute);
        }
        let replacement_error = if outcome == ExecutionOutcome::Failed {
            operation_context.error_response.take()
        } else {
            None
        };
        let remove_operation =
            outcome == ExecutionOutcome::Failed || operation_context.describe.is_none();
        if remove_operation {
            self.operations.remove(operation);
        }
        let metrics = execute
            .as_ref()
            .and_then(ExecuteContext::session_id)
            .and_then(|id| self.statement_metrics.get(&id).cloned());
        let finished_metrics = if outcome == ExecutionOutcome::Suspended {
            let execute = execute.as_ref().unwrap();
            self.suspended_executions
                .insert(execute.name.clone(), (*operation, execute.clone()));
            None
        } else {
            execute
                .as_ref()
                .and_then(ExecuteContext::session_id)
                .and_then(|id| self.statement_metrics.remove(&id))
        };
        Ok(ExecutionTransition {
            execute,
            metrics,
            finished_metrics,
            replacement_error,
            execution_finished: true,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionOutcome {
    Completed,
    Suspended,
    Failed,
}

struct ExecutionTransition {
    execute: Option<ExecuteContext>,
    metrics: Option<SessionMetricsContext>,
    finished_metrics: Option<SessionMetricsContext>,
    replacement_error: Option<DiagnosticResponse>,
    execution_finished: bool,
}

#[derive(Clone, Debug)]
pub struct SessionMetricsContext {
    id: SessionId,
    start: Instant,
    records_statement_metrics: bool,
    pub phase_timing: PhaseTiming,
    pub metadata: StatementMetadata,
}

impl SessionMetricsContext {
    fn new(id: SessionId) -> SessionMetricsContext {
        SessionMetricsContext {
            id,
            start: Instant::now(),
            records_statement_metrics: true,
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

impl<T> Context<T>
where
    T: EncryptionService,
{
    pub fn new(
        client_id: i32,
        config: Arc<TandemConfig>,
        encrypt_config: Arc<EncryptConfig>,
        schema: Arc<Schema>,
        upstream_tls_roots: Arc<rustls::RootCertStore>,
        encryption: T,
        reload_sender: ReloadSender,
    ) -> Context<T> {
        let schema_store =
            CommittedSchemaStore::from_parts((*schema).clone(), (*encrypt_config).clone());
        Self::new_with_schema_store(
            client_id,
            config,
            schema_store,
            upstream_tls_roots,
            encryption,
            reload_sender,
        )
    }

    /// Constructs a connection context over the shared committed schema store.
    pub fn new_with_schema_store(
        client_id: i32,
        config: Arc<TandemConfig>,
        schema_store: CommittedSchemaStore,
        upstream_tls_roots: Arc<rustls::RootCertStore>,
        encryption: T,
        reload_sender: ReloadSender,
    ) -> Context<T> {
        let schema_middleware = SchemaMiddleware::from_store(schema_store);

        Context {
            protocol_state: Arc::new(RwLock::new(ConnectionProtocolState::default())),
            upstream_tls_roots,
            client_id,
            config,
            schema_middleware,
            encryption,
            reload_sender,
            session_id_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn set_describe(&self, operation: OperationId, describe: Describe) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, describe = ?describe);
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let statement = match &describe {
            Describe {
                name,
                target: DescribeTarget::Portal,
            } => match state.portals.get(name).map(Arc::as_ref) {
                Some(Portal::Encrypted { statement, .. }) => Some(statement.clone()),
                Some(Portal::Passthrough { .. }) | None => None,
            },
            Describe {
                name,
                target: DescribeTarget::Statement,
            } => state.prepared_statements.statement(name),
        };
        state.operations.entry(operation).or_default().describe =
            Some(DescribeContext { statement });
        Ok(())
    }

    fn set_non_execution(&self, operation: OperationId) -> Result<(), Error> {
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .operations
            .entry(operation)
            .or_default();
        Ok(())
    }

    ///
    /// Marks the current Describe as complete
    /// Removes the Describe from the Queue
    ///
    pub fn complete_describe(&self, operation: OperationId) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Describe complete");
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let Some(context) = state.operations.get_mut(&operation) else {
            return Err(crate::error::ContextError::UnknownOperation.into());
        };
        if context.describe.take().is_none() {
            if context.execute.is_some() {
                return Ok(());
            }
            state.operations.remove(&operation);
            return Err(crate::error::ContextError::UnknownDescribe.into());
        }
        if context.execute.is_none() {
            state.operations.remove(&operation);
        }
        Ok(())
    }

    pub fn start_metrics_scope(&mut self) -> Result<SessionId, Error> {
        let id = SessionId(self.session_id_counter.fetch_add(1, Ordering::Relaxed));
        let ctx = SessionMetricsContext::new(id);
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .statement_metrics
            .insert(id, ctx);
        Ok(id)
    }

    pub fn finish_metrics_scope(&mut self, session_id: Option<SessionId>) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Session Metrics finished");

        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let metrics_scope = session_id.and_then(|id| state.statement_metrics.remove(&id));
        drop(state);
        self.record_finished_metrics_scope(metrics_scope);
        Ok(())
    }

    fn record_finished_metrics_scope(&self, metrics_scope: Option<SessionMetricsContext>) {
        if let Some(metrics_scope) = metrics_scope {
            if !metrics_scope.records_statement_metrics {
                return;
            }
            let duration = metrics_scope.duration();
            let metadata = &metrics_scope.metadata;

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
                let timing = &metrics_scope.phase_timing;

                // Increment slow statements counter
                counter!(SLOW_STATEMENTS_TOTAL).increment(1);

                let breakdown = json!({
                    "parse_ms": timing.parse_duration.map(|d| d.as_millis()),
                    "encrypt_ms": timing.encrypt_duration.map(|d| d.as_millis()),
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
    }

    fn set_execute(
        &mut self,
        operation: OperationId,
        name: Name,
        session_id: Option<SessionId>,
    ) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, execute = ?name);
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let portal = state.portals.get(&name).cloned();
        let execute = ExecuteContext::new(name, portal, session_id);
        state.operations.entry(operation).or_default().execute = Some(execute);
        Ok(())
    }

    fn set_simple_query_execute_until_ready(
        &mut self,
        operation: OperationId,
        name: Name,
        session_id: Option<SessionId>,
    ) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, execute = ?name);

        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let portal = state.portals.get(&name).cloned();
        let execute = ExecuteContext::new(name, portal, session_id).until_readiness();
        state.operations.entry(operation).or_default().execute = Some(execute);
        Ok(())
    }

    /// Set execute state for a portal, looking up its metrics scope ID internally.
    fn set_execute_for_portal(&mut self, operation: OperationId, name: Name) -> Result<(), Error> {
        let execution_id = SessionId(self.session_id_counter.fetch_add(1, Ordering::Relaxed));
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let portal = state.portals.get(&name).cloned();
        let template_id = portal.as_ref().and_then(|portal| portal.session_id());
        let execute = state
            .suspended_executions
            .remove(&name)
            .map(|(_, execute)| execute)
            .unwrap_or_else(|| {
                let mut metrics = template_id
                    .and_then(|id| state.statement_metrics.get(&id).cloned())
                    .unwrap_or_else(|| SessionMetricsContext::new(execution_id));
                metrics.id = execution_id;
                metrics.start = Instant::now();
                metrics.records_statement_metrics = true;
                state.statement_metrics.insert(execution_id, metrics);
                ExecuteContext::new(name.clone(), portal, Some(execution_id))
            });
        let deallocation = state
            .portal_operations
            .get(&name)
            .and_then(|owner| owner.deallocation.clone());
        let context = state.operations.entry(operation).or_default();
        context.execute = Some(execute);
        context.deallocations.extend(deallocation);
        Ok(())
    }

    /// Applies one terminal or suspended Execute outcome atomically.
    fn finish_execution(
        &self,
        operation: OperationId,
        outcome: ExecutionOutcome,
    ) -> Result<Option<DiagnosticResponse>, Error> {
        debug!(target: CONTEXT, client_id = self.client_id, ?outcome, msg = "Execute outcome");

        let transition = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            state.finish_execution(&operation, outcome)?
        };

        if transition.execution_finished && outcome != ExecutionOutcome::Suspended {
            if let Some(execute) = transition.execute.as_ref() {
                self.record_execution_duration(execute, transition.metrics.as_ref());

                if execute.name.is_empty() {
                    self.close_portal_if_current(&execute.name, execute.portal.as_ref())?;
                }
            }
        }
        self.record_finished_metrics_scope(transition.finished_metrics);
        Ok(transition.replacement_error)
    }

    fn record_execution_duration(
        &self,
        execute: &ExecuteContext,
        metrics: Option<&SessionMetricsContext>,
    ) {
        let (statement_type, protocol, mapped, multi_statement) = metrics
            .map(|metrics_scope| {
                let metadata = &metrics_scope.metadata;
                (
                    metadata
                        .statement_type
                        .map(|kind| kind.as_label())
                        .unwrap_or("unknown"),
                    metadata
                        .protocol
                        .map(|kind| kind.as_label())
                        .unwrap_or("unknown"),
                    if metadata.encrypted { "true" } else { "false" },
                    if metadata.multi_statement {
                        "true"
                    } else {
                        "false"
                    },
                )
            })
            .unwrap_or(("unknown", "unknown", "false", "false"));
        histogram!(
            STATEMENTS_EXECUTION_DURATION_SECONDS,
            "statement_type" => statement_type,
            "protocol" => protocol,
            "mapped" => mapped,
            "multi_statement" => multi_statement
        )
        .record(execute.duration());
    }

    #[cfg(test)]
    fn add_statement(&self, name: Name, statement: Statement) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .prepared_statements
            .set_statement(name, Arc::new(statement));
        Ok(())
    }

    fn close_statement(&self, name: &Name) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);

        let finished_metrics_scope = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            state.remove_statement_metadata(name)
        };
        self.record_finished_metrics_scope(finished_metrics_scope);
        Ok(())
    }

    /// Record that the SQL of statement `name` is a `DEALLOCATE` or `DISCARD ALL`, so that
    /// executing it through the extended protocol releases what it removes.
    pub fn set_statement_deallocation(
        &self,
        name: Name,
        deallocation: Deallocation,
    ) -> Result<(), Error> {
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .prepared_statements
            .set_deallocation(name, deallocation);
        Ok(())
    }

    /// Carry the deallocation of `statement`, if any, onto the portal just bound from it.
    pub fn bind_statement_effects(&self, portal: &Name, statement: &Name) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let deallocation = state.prepared_statements.deallocation(statement);
        if let Some(owner) = state.portal_operations.get_mut(portal) {
            owner.deallocation = deallocation;
        }
        Ok(())
    }

    /// Record the SQL deallocations in a simple Query.
    ///
    /// They are discarded with the operation if PostgreSQL never confirms them.
    pub fn set_deallocations(
        &self,
        operation: OperationId,
        deallocations: Vec<Deallocation>,
    ) -> Result<(), Error> {
        if deallocations.is_empty() {
            return Ok(());
        }
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .operations
            .entry(operation)
            .or_default()
            .deallocations
            .extend(deallocations);
        Ok(())
    }

    ///
    /// Handle a `CommandComplete` from PostgreSQL for a `DEALLOCATE` or `DISCARD ALL`.
    ///
    /// Applies the next deallocation in the operation's Query, removing the statements
    /// PostgreSQL has just removed. Without this, clients that deallocate with SQL (psycopg 3,
    /// Rails, pool reset queries) leave every statement they prepare behind for the life of the
    /// connection. Statements prepared by a later Parse keep their state even when they reuse
    /// a deallocated name.
    ///
    /// `all` is whether the tag removes every statement (`DEALLOCATE ALL`, `DISCARD ALL`). A
    /// queued deallocation of the other kind means Proxy is out of step with PostgreSQL, so the
    /// operation's queue is cleared and nothing is removed.
    ///
    pub fn complete_deallocation(&self, operation: OperationId, all: bool) -> Result<(), Error> {
        let finished_metrics = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            let Some(deallocations) = state
                .operations
                .get_mut(&operation)
                .map(|context| &mut context.deallocations)
            else {
                return Ok(());
            };
            let Some(deallocation) = deallocations.pop_front() else {
                return Ok(());
            };
            if matches!(deallocation, Deallocation::AllStatements) != all {
                deallocations.clear();
                warn!(
                    target: CONTEXT,
                    client_id = self.client_id,
                    msg = "Deallocation does not match the command tag, keeping statements",
                    ?deallocation,
                    all,
                );
                return Ok(());
            }

            debug!(target: CONTEXT, client_id = self.client_id, msg = "Deallocate", ?deallocation);

            let names = state.prepared_statements.deallocation_names(deallocation);
            for name in &names {
                // Also when a later Parse prepared the name again, so that Parse's rollback
                // cannot restore the statement PostgreSQL removed
                state.settings_protocol.deallocated(operation, name);
            }
            let candidates = state.prepared_statements.deallocate(&names, operation);
            state.take_unreferenced_metrics(candidates)
        };

        for metrics in finished_metrics {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    /// Names of the statements with a recorded Parse.
    #[cfg(test)]
    pub(crate) fn statement_operation_names(&self) -> Vec<String> {
        let state = self.protocol_state.read().unwrap();
        state.prepared_statements.parsed_names()
    }

    /// Size of every per-connection protocol state collection.
    ///
    /// Test-only accessor used to assert per-connection state stays bounded.
    #[cfg(test)]
    pub(crate) fn protocol_state_sizes(&self) -> ProtocolStateSizes {
        let state = self.protocol_state.read().unwrap();
        ProtocolStateSizes {
            operations: state.operations.len(),
            statement_metrics: state.statement_metrics.len(),
            suspended_executions: state.suspended_executions.len(),
            prepared_statements: state.prepared_statements.len(),
            settings_statements: state.settings_protocol.statement_count(),
            portals: state.portals.len(),
            portal_operations: state.portal_operations.len(),
        }
    }

    fn ready_for_query(
        &self,
        status: TransactionStatus,
        boundary: Option<OperationId>,
    ) -> Result<(), Error> {
        let (finished_executions, finished_metrics_scopes) = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            state.settings.ready_for_query(status);
            let mut candidates = state
                .settings_protocol
                .ready(boundary, status == TransactionStatus::Idle);
            let Some(boundary) = boundary else {
                return Ok(());
            };
            let mut executions = Vec::new();
            if status == TransactionStatus::Idle {
                let portal_names = state
                    .portal_operations
                    .iter()
                    .filter_map(|(name, owner)| {
                        (owner.operation <= boundary).then_some(name.clone())
                    })
                    .collect::<Vec<_>>();
                for name in portal_names {
                    state.portal_operations.remove(&name);
                    candidates.extend(
                        state
                            .portals
                            .remove(&name)
                            .and_then(|portal| portal.session_id()),
                    );
                }

                let suspended_names = state
                    .suspended_executions
                    .iter()
                    .filter_map(|(name, (operation, _))| {
                        (*operation <= boundary).then_some(name.clone())
                    })
                    .collect::<Vec<_>>();
                for name in suspended_names {
                    candidates.extend(
                        state
                            .suspended_executions
                            .remove(&name)
                            .and_then(|(_, execute)| execute.session_id()),
                    );
                }
            }

            let operation_ids = state
                .operations
                .keys()
                .filter(|operation| **operation <= boundary)
                .copied()
                .collect::<Vec<_>>();
            for operation in operation_ids {
                if let Some(execute) = state
                    .operations
                    .remove(&operation)
                    .and_then(|operation| operation.execute)
                {
                    let metrics = execute
                        .session_id()
                        .and_then(|id| state.statement_metrics.get(&id).cloned());
                    candidates.extend(execute.session_id());
                    executions.push((execute, metrics));
                }
            }
            (executions, state.take_unreferenced_metrics(candidates))
        };
        for (execute, metrics) in finished_executions {
            self.record_execution_duration(&execute, metrics.as_ref());
            if execute.completes_at_readiness && execute.name.is_empty() {
                self.close_portal_if_current(&execute.name, execute.portal.as_ref())?;
            }
        }
        for metrics_scope in finished_metrics_scopes {
            self.record_finished_metrics_scope(Some(metrics_scope));
        }
        Ok(())
    }

    /// Close a statement explicitly requested by the client.
    ///
    /// PostgreSQL portals retain the parsed statement they reference and remain
    /// valid after the statement name is closed, so they must not be removed.
    #[cfg(test)]
    fn close_statement_explicit(&self, name: &Name) -> Result<(), Error> {
        self.close_statement(name)
    }

    pub fn discard_operation(&mut self, operation: OperationId) -> Result<(), Error> {
        let finished_metrics_scopes = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            let metrics_scope_id = state
                .operations
                .remove(&operation)
                .and_then(|operation| operation.execute)
                .and_then(|execute| execute.session_id());
            state.take_unreferenced_metrics(metrics_scope_id)
        };
        for metrics_scope in finished_metrics_scopes {
            self.record_finished_metrics_scope(Some(metrics_scope));
        }
        Ok(())
    }

    fn set_operation_error(
        &mut self,
        operation: OperationId,
        response: DiagnosticResponse,
    ) -> Result<(), Error> {
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .operations
            .entry(operation)
            .or_default()
            .error_response = Some(response);
        Ok(())
    }

    fn add_portal(&self, operation: OperationId, name: Name, portal: Portal) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, name = ?name, portal = ?portal);
        let finished_metrics = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            state.install_portal(operation, name, portal)
        };
        for metrics in finished_metrics {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    /// Whether a Parse on this connection prepared `name`.
    pub fn is_prepared(&self, name: &Name) -> Result<bool, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state.prepared_statements.contains(name))
    }

    pub fn get_statement(&self, name: &Name) -> Result<Option<Arc<Statement>>, Error> {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state.prepared_statements.statement(name))
    }

    #[cfg(test)]
    fn set_statement_metrics_scope(
        &mut self,
        name: Name,
        session_id: SessionId,
    ) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        if let Some(metrics) = state.statement_metrics.get_mut(&session_id) {
            metrics.records_statement_metrics = false;
        }
        state
            .prepared_statements
            .set_metrics_scope(name, session_id);
        Ok(())
    }

    fn get_statement_metrics_scope(&self, name: &Name) -> Result<Option<SessionId>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state.prepared_statements.metrics_scope(name))
    }

    fn start_portal_metrics_scope(&mut self, statement: &Name) -> Result<Option<SessionId>, Error> {
        let id = SessionId(self.session_id_counter.fetch_add(1, Ordering::Relaxed));
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let Some(template_id) = state.prepared_statements.metrics_scope(statement) else {
            return Ok(None);
        };
        let Some(mut metrics) = state.statement_metrics.get(&template_id).cloned() else {
            return Err(crate::error::ContextError::StatementMetricsUnavailable.into());
        };
        metrics.id = id;
        metrics.start = Instant::now();
        metrics.records_statement_metrics = false;
        state.statement_metrics.insert(id, metrics);
        Ok(Some(id))
    }

    ///
    /// Close the portal identified by `name`
    /// Portal is removed from  queue
    ///
    #[cfg(test)]
    fn close_portal(&self, name: &Name) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Close Portal", name = ?name);
        let finished_metrics = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            state.portal_operations.remove(name);
            let mut candidates = Vec::new();
            candidates.extend(
                state
                    .portals
                    .remove(name)
                    .and_then(|portal| portal.session_id()),
            );
            candidates.extend(
                state
                    .suspended_executions
                    .remove(name)
                    .and_then(|(_, execute)| execute.session_id()),
            );
            state.take_unreferenced_metrics(candidates)
        };
        for metrics in finished_metrics {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    fn close_portal_if_current(
        &self,
        name: &Name,
        expected: Option<&Arc<Portal>>,
    ) -> Result<(), Error> {
        let finished_metrics = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            if !expected.is_some_and(|expected| {
                state
                    .portals
                    .get(name)
                    .is_some_and(|current| Arc::ptr_eq(current, expected))
            }) {
                return Ok(());
            }
            state.portal_operations.remove(name);
            // The portal's metrics scope is released with it, as in `close_portal`
            let candidates = state
                .portals
                .remove(name)
                .and_then(|portal| portal.session_id());
            state.take_unreferenced_metrics(candidates)
        };
        for metrics in finished_metrics {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    pub fn get_portal(&self, name: &Name) -> Result<Option<Arc<Portal>>, Error> {
        debug!(target: CONTEXT, client_id = self.client_id, src = "Get Portal", portal = ?name);
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state.portals.get(name).cloned())
    }

    pub fn get_portal_statement(&self, name: &Name) -> Result<Option<Arc<Statement>>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let Some(portal) = state.portals.get(name) else {
            return Ok(None);
        };

        debug!(target: CONTEXT, client_id = self.client_id, portal = ?portal);

        Ok(match portal.as_ref() {
            Portal::Encrypted { statement, .. } => Some(statement.clone()),
            Portal::Passthrough { .. } => None,
        })
    }

    pub fn get_portal_metrics_scope_id(&self, name: &Name) -> Result<Option<SessionId>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state
            .portals
            .get(name)
            .and_then(|portal| portal.session_id()))
    }

    pub fn get_statement_for_operation(
        &self,
        operation: OperationId,
    ) -> Result<Option<Arc<Statement>>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let Some(context) = state.operations.get(&operation) else {
            return Err(crate::error::ContextError::UnknownOperation.into());
        };
        if let Some(statement) = context
            .describe
            .as_ref()
            .and_then(|describe| describe.statement.as_ref())
        {
            return Ok(Some(statement.clone()));
        }
        Ok(
            match context
                .execute
                .as_ref()
                .and_then(|execute| execute.portal.as_deref())
            {
                Some(Portal::Encrypted { statement, .. }) => Some(statement.clone()),
                Some(Portal::Passthrough { .. }) | None => None,
            },
        )
    }

    pub fn get_statement_from_describe(
        &self,
        operation: OperationId,
    ) -> Result<Option<Arc<Statement>>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let context = state
            .operations
            .get(&operation)
            .ok_or(crate::error::ContextError::UnknownOperation)?;
        Ok(context
            .describe
            .as_ref()
            .and_then(|describe| describe.statement.clone()))
    }

    pub fn get_portal_from_execute(
        &self,
        operation: OperationId,
    ) -> Result<Option<Arc<Portal>>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let context = state
            .operations
            .get(&operation)
            .ok_or(crate::error::ContextError::UnknownOperation)?;
        let execute = context
            .execute
            .as_ref()
            .ok_or(crate::error::ContextError::OperationWithoutExecute)?;
        Ok(execute.portal.clone())
    }

    pub fn get_execute(&self, operation: OperationId) -> Result<Option<ExecuteContext>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let operation_context = state
            .operations
            .get(&operation)
            .ok_or(crate::error::ContextError::UnknownOperation)?;
        let execute_context = operation_context
            .execute
            .as_ref()
            .ok_or(crate::error::ContextError::OperationWithoutExecute)?;
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Get Execute", execute = ?execute_context);
        Ok(Some(execute_context.to_owned()))
    }

    pub fn get_metrics_scope(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionMetricsContext>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let Some(metrics_scope) = state.statement_metrics.get(&session_id) else {
            return Ok(None);
        };
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Get statement metrics scope", statement_metrics = ?metrics_scope);
        Ok(Some(metrics_scope.to_owned()))
    }

    #[cfg(test)]
    pub fn active_metrics_scopes(&self) -> Result<usize, Error> {
        self.protocol_state
            .read()
            .map(|state| state.statement_metrics.len())
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable.into())
    }

    /// Returns the resolver for this connection's effective schema snapshot.
    pub fn get_table_resolver(&self) -> Arc<TableResolver> {
        self.schema_middleware.resolver()
    }

    /// Records schema intent for a parsed prepared statement.
    fn prepare_schema_statement(&self, name: Name, statement: sqltk::parser::ast::Statement) {
        self.schema_middleware.prepare(name, statement);
    }

    /// Associates a bound portal with its prepared statement's schema intent.
    fn bind_schema_statement(&self, portal: Name, prepared_statement: &Name) {
        self.schema_middleware.bind(portal, prepared_statement);
    }

    /// Records a portal execution and returns whether its DDL needs an injected flush.
    fn execute_schema_portal(&self, portal: &Name) -> bool {
        self.schema_middleware.execute(portal)
    }

    /// Records statements in one simple-query protocol message.
    fn execute_simple_schema_statements(&self, statements: &[sqltk::parser::ast::Statement]) {
        self.schema_middleware.simple_query(statements);
    }

    /// Returns whether a simple-query batch must be rejected to protect encryption.
    pub fn simple_query_requires_fail_closed(
        &self,
        statements: &[sqltk::parser::ast::Statement],
    ) -> bool {
        self.schema_middleware
            .simple_query_requires_fail_closed(statements)
    }

    /// Records an extended-protocol synchronization boundary.
    fn mark_schema_protocol_boundary(&self) {
        self.schema_middleware.protocol_boundary();
    }

    /// Waits for preceding schema-changing executions to resolve.
    pub async fn wait_for_schema_execution(&self) {
        self.schema_middleware.wait_for_ddl().await;
    }

    fn report_schema_execution_succeeded(&self) {
        self.schema_middleware.execution_succeeded();
    }

    fn report_schema_execution_failed(&self) {
        self.schema_middleware.execution_failed();
    }

    /// Returns whether a schema-changing execution is awaiting a backend outcome.
    pub fn schema_ddl_in_flight(&self) -> bool {
        self.schema_middleware.ddl_in_flight()
    }

    /// Refuses schema-dependent work after confirmed unmodelled DDL.
    pub fn ensure_schema_modelled(&self) -> Result<(), Error> {
        if self.schema_middleware.has_unmodelled_ddl() {
            return Err(crate::error::MappingError::UnmodelledDdl.into());
        }
        Ok(())
    }

    /// Replaces idle connection-local state with the latest committed snapshot.
    pub fn adopt_latest_schema(&self) {
        self.schema_middleware.adopt_latest();
    }

    /// Publishes pending shared state, then prepares the effective schema for mapping.
    pub async fn prepare_schema_for_statement(&self) -> Result<(), Error> {
        if self
            .schema_middleware
            .requires_publication_before_statement()
        {
            if !self.reload_schema().await {
                return Err(ConfigError::SchemaCouldNotBeLoaded.into());
            }
            self.schema_middleware.publication_succeeded();
        }
        self.schema_middleware.before_statement();
        Ok(())
    }

    /// Reports a readiness boundary and PostgreSQL transaction status.
    fn schema_ready_for_query(&self, status: crate::proxy::schema::TransactionStatus) {
        self.schema_middleware.ready_for_query(status);
    }

    fn prepare_settings_statement(
        &self,
        operation: OperationId,
        name: Name,
        statement: &sqltk::parser::ast::Statement,
    ) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let settings = settings_protocol::EffectiveSettings::new(&state.settings);
        state
            .settings_protocol
            .prepare(operation, name, statement, settings)
    }

    fn bind_settings_statement(
        &self,
        operation: OperationId,
        portal: Name,
        name: &Name,
    ) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let settings = settings_protocol::EffectiveSettings::new(&state.settings);
        let encrypted_values = state
            .prepared_statements
            .statement(name)
            .is_some_and(|statement| statement.param_columns.iter().any(Option::is_some));
        let metadata = state.portal_snapshot(&portal);
        state
            .settings_protocol
            .bind(
                operation,
                portal,
                name,
                settings,
                encrypted_values,
                metadata,
            )
            .map_err(|refusal| self.prepared_settings_changed(refusal))
    }

    fn prepared_settings_changed(&self, refusal: settings_protocol::SettingsRefusal) -> Error {
        warn!(
            target: CONTEXT,
            client_id = self.client_id,
            refusal = ?refusal,
            "Prepared statement refused after effective settings changed"
        );
        crate::error::MappingError::PreparedSettingsChanged.into()
    }

    fn execute_settings_portal(
        &self,
        operation: OperationId,
        portal: &Name,
    ) -> Result<bool, Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let ConnectionProtocolState {
            settings,
            settings_protocol,
            ..
        } = &mut *state;
        settings_protocol
            .execute(operation, portal, settings)
            .map_err(|refusal| self.prepared_settings_changed(refusal))
    }

    fn execute_simple_settings_statements(
        &self,
        operation: OperationId,
        statements: &[sqltk::parser::ast::Statement],
    ) -> Result<bool, Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let ConnectionProtocolState {
            settings,
            settings_protocol,
            ..
        } = &mut *state;
        settings_protocol.simple_query(operation, statements, settings)
    }

    pub fn settings_simple_query_requires_fail_closed(
        &self,
        statements: &[sqltk::parser::ast::Statement],
    ) -> Result<bool, Error> {
        let Ok(state) = self.protocol_state.read() else {
            return Ok(true);
        };
        let mut changed = false;
        for (statement, _, changes) in
            settings_protocol::project_settings_effects(&state.settings, statements)?
        {
            if changed && eql_mapper::requires_type_check(statement) {
                return Ok(true);
            }
            changed |= changes;
        }
        Ok(false)
    }

    pub fn discarding_extended_messages(&self) -> Result<bool, Error> {
        Ok(self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .settings_protocol
            .discarding())
    }

    fn settings_protocol_boundary(&self, operation: OperationId) -> Result<(), Error> {
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .settings_protocol
            .sync(operation);
        Ok(())
    }

    fn settings_execution_succeeded(
        &self,
        operation: Option<OperationId>,
        tag: &str,
    ) -> Result<(), Error> {
        let Some(operation) = operation else {
            return Ok(());
        };
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let mut finished = Vec::new();
        if let Some(statement) = state.settings_protocol.succeeded(operation) {
            state.settings.command_succeeded(&statement, tag);
            if matches!(
                statement,
                settings::SettingsCommand::Change(settings::SettingChange::DiscardAll)
            ) {
                let ConnectionProtocolState {
                    settings_protocol,
                    prepared_statements,
                    portals,
                    portal_operations,
                    ..
                } = &mut *state;
                prepared_statements.retain(|name| settings_protocol.has_statement(name));
                portals.retain(|name, _| settings_protocol.has_portal(name));
                portal_operations.retain(|name, _| settings_protocol.has_portal(name));
                let candidates = state.statement_metrics.keys().copied().collect::<Vec<_>>();
                finished = state.take_unreferenced_metrics(candidates);
            }
        }
        drop(state);
        for metrics in finished {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    fn settings_execution_failed(&self, operation: Option<OperationId>) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let finished = state.restore_failed_preparation(operation);
        drop(state);
        for metrics in finished {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    pub fn settings_in_flight(&self) -> Result<bool, Error> {
        Ok(self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .settings_protocol
            .in_flight())
    }

    pub fn unsafe_disable_mapping(&self) -> Result<bool, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state.settings.unsafe_disable_mapping())
    }

    pub fn keyset_identifier(&self) -> Result<Option<KeysetIdentifier>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state.settings.keyset())
    }

    // Service delegation methods
    pub async fn encrypt(
        &self,
        plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
        if plaintexts.iter().all(Option::is_none) {
            return Ok(std::iter::repeat_with(|| None)
                .take(plaintexts.len())
                .collect());
        }

        let keyset_id = self.keyset_identifier()?;
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
        let keyset_id = self.keyset_identifier()?;
        self.encryption.decrypt(keyset_id, ciphertexts).await
    }

    pub async fn decrypt_inbound_eql(
        &self,
        ciphertexts: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
        let keyset_id = self.keyset_identifier()?;
        self.encryption
            .decrypt_inbound_eql(keyset_id, ciphertexts)
            .await
    }

    pub async fn reload_schema(&self) -> bool {
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
                return false;
            }
        }

        debug!(target: CONTEXT, msg = "Waiting for schema reload");
        let response = receiver.await;
        debug!(target: CONTEXT, msg = "Database schema reloaded", ?response);
        matches!(response, Ok(true))
    }

    /// Reload schema if it has changed since last check.
    pub async fn publish_schema_if_changed(&self) -> Result<(), Error> {
        if !self.schema_middleware.needs_publication() {
            self.adopt_latest_schema();
            return Ok(());
        }

        if self.schema_middleware.has_local_changes() {
            self.schema_middleware.mark_publication_pending();
        }

        if !self.reload_schema().await {
            return Err(ConfigError::SchemaCouldNotBeLoaded.into());
        }

        self.schema_middleware.publication_succeeded();
        Ok(())
    }

    pub fn is_passthrough(&self) -> bool {
        self.schema_middleware.encrypt_config().is_empty() || self.config.mapping_disabled()
    }

    // Column processing delegation methods
    pub fn get_projection_columns(
        &self,
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        ColumnMapper::new(self.schema_middleware.encrypt_config())
            .get_projection_columns(typed_statement)
    }

    pub fn get_param_columns(
        &self,
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        ColumnMapper::new(self.schema_middleware.encrypt_config())
            .get_param_columns(typed_statement)
    }

    pub fn get_output_param_columns(
        &self,
        plan: &eql_mapper::ParamPlan,
    ) -> Result<Vec<Option<Column>>, Error> {
        ColumnMapper::new(self.schema_middleware.encrypt_config()).get_output_param_columns(plan)
    }

    pub fn get_literal_columns(
        &self,
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        ColumnMapper::new(self.schema_middleware.encrypt_config())
            .get_literal_columns(typed_statement)
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

    pub(crate) fn upstream_tls_roots(&self) -> Arc<rustls::RootCertStore> {
        self.upstream_tls_roots.clone()
    }

    fn with_statement_metrics_scope_mut<F>(
        &mut self,
        session_id: SessionId,
        f: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(&mut SessionMetricsContext),
    {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        if let Some(metrics_scope) = state.statement_metrics.get_mut(&session_id) {
            f(metrics_scope);
        }
        Ok(())
    }

    /// Record parse phase duration for the statement metrics scope (first write wins)
    pub fn record_parse_duration(
        &mut self,
        session_id: SessionId,
        duration: Duration,
    ) -> Result<(), Error> {
        self.with_statement_metrics_scope_mut(session_id, |metrics_scope| {
            metrics_scope.phase_timing.record_parse(duration);
        })
    }

    /// Add encrypt phase duration for the statement metrics scope (accumulate)
    pub fn add_encrypt_duration(
        &mut self,
        session_id: SessionId,
        duration: Duration,
    ) -> Result<(), Error> {
        self.with_statement_metrics_scope_mut(session_id, |metrics_scope| {
            metrics_scope.phase_timing.add_encrypt(duration);
        })
    }

    /// Add decrypt phase duration (accumulate)
    pub fn add_decrypt_duration(
        &mut self,
        session_id: SessionId,
        duration: Duration,
    ) -> Result<(), Error> {
        self.with_statement_metrics_scope_mut(session_id, |metrics_scope| {
            metrics_scope.phase_timing.add_decrypt(duration);
        })
    }

    /// Update metadata for a statement metrics scope.
    pub fn update_statement_metadata<F>(&mut self, session_id: SessionId, f: F) -> Result<(), Error>
    where
        F: FnOnce(&mut StatementMetadata),
    {
        self.with_statement_metrics_scope_mut(session_id, |metrics_scope| {
            f(&mut metrics_scope.metadata);
        })
    }

    /// Update statement metadata if a metrics scope ID is present, no-op otherwise.
    pub fn with_metrics_scope<F>(
        &mut self,
        session_id: Option<SessionId>,
        f: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(&mut SessionMetricsContext),
    {
        if let Some(sid) = session_id {
            self.with_statement_metrics_scope_mut(sid, f)?;
        }
        Ok(())
    }

    /// Add decrypt phase duration for the current execution metrics scope (if any)
    pub fn add_decrypt_duration_for_execute(
        &mut self,
        operation: OperationId,
        duration: Duration,
    ) -> Result<(), Error> {
        let session_id = self
            .get_execute(operation)?
            .and_then(|execute| execute.session_id());
        if let Some(session_id) = session_id {
            self.add_decrypt_duration(session_id, duration)?;
        }
        Ok(())
    }
}

// Low-level fixtures for adapter tests that inject a partially observed protocol.
#[cfg(test)]
impl<T: EncryptionService> Context<T> {
    pub fn test_execute_simple_schema_statements(
        &self,
        statements: &[sqltk::parser::ast::Statement],
    ) {
        self.execute_simple_schema_statements(statements)
    }
    pub fn test_report_schema_execution_succeeded(&self) {
        self.report_schema_execution_succeeded()
    }
    pub fn test_finish_execution(
        &self,
        operation: OperationId,
        outcome: ExecutionOutcome,
    ) -> Result<Option<DiagnosticResponse>, Error> {
        self.finish_execution(operation, outcome)
    }
    pub fn test_set_operation_error(
        &mut self,
        operation: OperationId,
        response: DiagnosticResponse,
    ) -> Result<(), Error> {
        self.set_operation_error(operation, response)
    }
    pub fn test_set_statement_metrics_scope(
        &mut self,
        name: Name,
        session_id: SessionId,
    ) -> Result<(), Error> {
        self.set_statement_metrics_scope(name, session_id)
    }
    pub fn test_add_statement(&self, name: Name, statement: Statement) -> Result<(), Error> {
        self.add_statement(name, statement)
    }
    pub fn test_ready_for_query(
        &self,
        status: TransactionStatus,
        boundary: Option<OperationId>,
    ) -> Result<(), Error> {
        self.ready_for_query(status, boundary)
    }
    pub fn test_set_execute(
        &mut self,
        operation: OperationId,
        name: Name,
        metrics: Option<SessionId>,
    ) -> Result<(), Error> {
        self.set_execute(operation, name, metrics)
    }
    pub fn test_statement_metrics_scope(&self, name: &Name) -> Result<Option<SessionId>, Error> {
        self.get_statement_metrics_scope(name)
    }
    pub fn test_non_execution(&self, operation: OperationId) -> Result<(), Error> {
        self.set_non_execution(operation)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConnectionProtocolState, Context, ExecuteContext, ExecutionEnd, ExecutionOutcome,
        KeysetIdentifier, OperationContext, Portal, SessionId, SessionMetricsContext, Statement,
    };
    use crate::{
        config::LogConfig,
        error::Error,
        log,
        postgresql::{rewrite::Name, test_operation_id as operation_id, Column, OperationId},
        proxy::{EncryptConfig, EncryptionService},
        TandemConfig,
    };
    use eql_mapper::Schema;
    use pg_proto::{Describe, DescribeTarget, TransactionStatus};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct TestService {}

    #[async_trait::async_trait]
    impl EncryptionService for TestService {
        async fn encrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
            _columns: &[Option<Column>],
        ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
            Ok(vec![])
        }

        async fn decrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(vec![])
        }

        async fn decrypt_inbound_eql(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn unavailable_settings_refuse_encryption_instead_of_using_default_keyset() {
        let context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();
        // The service fake succeeds if called; inaccessible settings must stop
        // the request before it can encrypt under an unintended default keyset.
        let result = context
            .encrypt(
                vec![Some(cipherstash_client::encryption::Plaintext::Text(Some(
                    "secret".to_owned(),
                )))],
                &[None],
            )
            .await;
        assert!(matches!(
            result,
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn suspended_execution_retains_its_metrics_scope_until_resumed_completion() {
        let session_id = SessionId(1);
        let mut state = ConnectionProtocolState::<u64>::default();
        state
            .statement_metrics
            .insert(session_id, SessionMetricsContext::new(session_id));
        state.operations.insert(
            1,
            OperationContext {
                execute: Some(ExecuteContext::new(Name::new(), None, Some(session_id))),
                ..OperationContext::default()
            },
        );

        let suspended = state
            .finish_execution(&1, ExecutionOutcome::Suspended)
            .unwrap();

        assert!(suspended.finished_metrics.is_none());
        assert!(state.statement_metrics.contains_key(&session_id));
        assert!(!state.operations.contains_key(&1));

        state.operations.insert(
            2,
            OperationContext {
                execute: Some(ExecuteContext::new(Name::new(), None, Some(session_id))),
                ..OperationContext::default()
            },
        );
        let completed = state
            .finish_execution(&2, ExecutionOutcome::Completed)
            .unwrap();

        assert_eq!(
            completed.finished_metrics.map(|metrics| metrics.id()),
            Some(session_id)
        );
        assert!(!state.statement_metrics.contains_key(&session_id));
        assert!(!state.operations.contains_key(&2));
    }

    #[test]
    fn each_extended_execution_records_its_own_statement_metrics() {
        let mut context = create_context();
        let statement = Name::from("statement");
        let portal = Name::from("portal");
        let template_scope = context.start_metrics_scope().unwrap();
        context
            .set_statement_metrics_scope(statement, template_scope)
            .unwrap();
        context
            .add_portal(
                operation_id(),
                portal.clone(),
                Portal::passthrough(Some(template_scope)),
            )
            .unwrap();
        let operation = operation_id();

        context.set_execute_for_portal(operation, portal).unwrap();

        let execution_scope = context
            .get_execute(operation)
            .unwrap()
            .unwrap()
            .session_id()
            .unwrap();
        assert!(
            context
                .get_metrics_scope(execution_scope)
                .unwrap()
                .unwrap()
                .records_statement_metrics
        );
    }

    #[test]
    fn execution_transition_rejects_stale_and_non_execute_operations() {
        let mut state = ConnectionProtocolState::<u64>::default();

        assert!(matches!(
            state.finish_execution(&1, ExecutionOutcome::Completed),
            Err(crate::error::ContextError::UnknownOperation)
        ));

        state.operations.insert(1, OperationContext::default());
        assert!(matches!(
            state.finish_execution(&1, ExecutionOutcome::Completed),
            Err(crate::error::ContextError::OperationWithoutExecute)
        ));
    }

    #[test]
    fn failed_execution_rejects_an_unknown_operation() {
        let context = create_context();

        assert!(matches!(
            context.finish_execution(operation_id(), ExecutionOutcome::Failed),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[test]
    fn adding_a_statement_fails_when_protocol_state_is_unavailable() {
        let context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.add_statement(
                Name::new(),
                Statement::new(vec![], vec![], vec![], vec![], vec![]),
            ),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn adding_a_portal_fails_when_protocol_state_is_unavailable() {
        let context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.add_portal(operation_id(), Name::new(), Portal::passthrough(None)),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn associating_statement_metrics_fails_when_protocol_state_is_unavailable() {
        let mut context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.set_statement_metrics_scope(Name::new(), SessionId(1)),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn starting_metrics_fails_when_protocol_state_is_unavailable() {
        let mut context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.start_metrics_scope(),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn finishing_metrics_fails_when_protocol_state_is_unavailable() {
        let mut context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.finish_metrics_scope(None),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn reading_protocol_metadata_fails_when_protocol_state_is_unavailable() {
        let context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.get_statement(&Name::from("statement")),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn closing_a_statement_fails_when_protocol_state_is_unavailable() {
        let context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.close_statement(&Name::from("statement")),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn completing_an_unknown_describe_rejects_the_stale_operation() {
        let context = create_context();

        assert!(matches!(
            context.complete_describe(operation_id()),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[test]
    fn completing_an_operation_without_describe_rejects_and_removes_it() {
        let mut context = create_context();
        let operation = operation_id();
        context
            .set_operation_error(
                operation,
                crate::postgresql::diagnostics::invalid_sql_statement("proxy error".to_owned()),
            )
            .unwrap();

        assert!(matches!(
            context.complete_describe(operation),
            Err(Error::Context(crate::error::ContextError::UnknownDescribe))
        ));
        assert!(matches!(
            context.complete_describe(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[test]
    fn row_description_for_an_execution_does_not_require_describe_state() {
        let mut context = create_context();
        let operation = operation_id();
        context.set_execute(operation, Name::new(), None).unwrap();

        context.complete_describe(operation).unwrap();

        assert!(context.get_execute(operation).unwrap().is_some());
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
            Arc::new(rustls::RootCertStore::empty()),
            service,
            reload_sender,
        )
    }

    async fn prepare_lifecycle(
        context: &mut Context<TestService>,
        name: &Name,
        sql: &str,
        mapped: Option<Statement>,
    ) -> OperationId {
        let operation = operation_id();
        context.begin_parse(operation, name.clone()).unwrap();
        let statement = crate::postgresql::parser::SqlParser::parse_statement(sql).unwrap();
        context
            .prepare_parse(operation, &statement, sql)
            .await
            .unwrap();
        context.finish_parse(operation, mapped).unwrap();
        operation
    }

    #[tokio::test]
    async fn lifecycle_preparation_does_not_activate_settings_and_requires_ordered_phases() {
        let mut context = create_context();
        let name = Name::from_static(b"setting");
        assert!(context.finish_parse(operation_id(), None).is_err());
        let parse = prepare_lifecycle(
            &mut context,
            &name,
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            None,
        )
        .await;
        assert!(!context.unsafe_disable_mapping().unwrap());
        assert!(context.finish_parse(parse, None).is_err());
        context.preparation_acknowledged(parse).unwrap();
        let bind = operation_id();
        let knowledge = context.begin_bind(bind, name.clone(), &name).unwrap();
        context
            .finish_bind(bind, Portal::passthrough(knowledge.metrics))
            .unwrap();
        context.preparation_acknowledged(bind).unwrap();
        assert!(!context.unsafe_disable_mapping().unwrap());
        let execute = operation_id();
        assert!(context.execute(execute, &name).unwrap());
        assert!(!context.unsafe_disable_mapping().unwrap());
        context
            .execution_completed(Some(execute), ExecutionEnd::Command("SET"), false)
            .unwrap();
        assert!(context.unsafe_disable_mapping().unwrap());
    }

    #[tokio::test]
    async fn lifecycle_later_batch_survives_failed_replacement_then_restores_original_metadata() {
        let mut context = create_context();
        let name = Name::from_static(b"statement");
        let original = prepare_lifecycle(&mut context, &name, "SELECT 1", Some(statement())).await;
        context.preparation_acknowledged(original).unwrap();
        let original_metadata = context.get_statement(&name).unwrap().unwrap();
        let rejected = prepare_lifecycle(&mut context, &name, "SELECT 2", Some(statement())).await;
        context.sync(operation_id()).unwrap();
        let later = prepare_lifecycle(&mut context, &name, "SELECT 3", Some(statement())).await;
        let later_metadata = context.get_statement(&name).unwrap().unwrap();
        context.postgres_failed(Some(rejected), false).unwrap();
        // Rebasing the later snapshot releases the rejected Parse's scope.
        assert_eq!(context.active_metrics_scopes().unwrap(), 2);
        assert_eq!(
            context.get_statement(&name).unwrap().unwrap(),
            later_metadata
        );
        assert!(Arc::ptr_eq(
            &context.get_statement(&name).unwrap().unwrap(),
            &later_metadata
        ));
        context.postgres_failed(Some(later), false).unwrap();
        assert!(Arc::ptr_eq(
            &context.get_statement(&name).unwrap().unwrap(),
            &original_metadata
        ));
        assert_eq!(context.active_metrics_scopes().unwrap(), 1);
        assert!(context.discarding_extended_messages().unwrap());
    }

    #[tokio::test]
    async fn lifecycle_local_failure_does_not_invent_a_postgres_failure() {
        let mut context = create_context();
        let operation = operation_id();
        let metrics = context.start_metrics_scope().unwrap();
        let setting = crate::postgresql::parser::SqlParser::parse_statement(
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
        )
        .unwrap();
        context
            .register_query(
                operation,
                metrics,
                Portal::passthrough(Some(metrics)),
                &[setting],
            )
            .unwrap();
        context.local_execution_failed(Some(operation)).unwrap();
        assert!(!context.unsafe_disable_mapping().unwrap());
        // A suppressed PostgreSQL success still activates its settings.
        context
            .execution_completed(Some(operation), ExecutionEnd::Command("SET"), true)
            .unwrap();
        assert!(context.unsafe_disable_mapping().unwrap());
        context
            .became_ready(Some(operation), TransactionStatus::Idle)
            .await
            .unwrap();
        assert!(context.get_metrics_scope(metrics).unwrap().is_none());
    }

    #[tokio::test]
    async fn lifecycle_suspension_keeps_one_execution_scope_and_reexecution_gets_another() {
        let mut context = create_context();
        let name = Name::from_static(b"query");
        let parse = prepare_lifecycle(&mut context, &name, "SELECT 1", Some(statement())).await;
        context.preparation_acknowledged(parse).unwrap();
        let bind = operation_id();
        let knowledge = context.begin_bind(bind, name.clone(), &name).unwrap();
        context
            .finish_bind(bind, Portal::passthrough(knowledge.metrics))
            .unwrap();
        context.preparation_acknowledged(bind).unwrap();
        let first = operation_id();
        context.execute(first, &name).unwrap();
        let first_scope = context.get_execute(first).unwrap().unwrap().session_id();
        context
            .execution_completed(Some(first), ExecutionEnd::Suspended, false)
            .unwrap();
        let resumed = operation_id();
        context.execute(resumed, &name).unwrap();
        assert_eq!(
            context.get_execute(resumed).unwrap().unwrap().session_id(),
            first_scope
        );
        context
            .execution_completed(Some(resumed), ExecutionEnd::Command("SELECT 1"), false)
            .unwrap();
        let repeated = operation_id();
        context.execute(repeated, &name).unwrap();
        assert_ne!(
            context.get_execute(repeated).unwrap().unwrap().session_id(),
            first_scope
        );
        assert!(context
            .get_metrics_scope(first_scope.unwrap())
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn pipelined_batch_failure_rebases_analysed_metadata_without_replacing_later_work() {
        let mut context = create_context();
        let name = Name::from_static(b"same_name");
        let original = prepare_lifecycle(&mut context, &name, "SELECT 1", Some(statement())).await;
        let original_metadata = context.get_statement(&name).unwrap().unwrap();
        context.preparation_acknowledged(original).unwrap();
        let rejected = prepare_lifecycle(&mut context, &name, "SELECT 1", Some(statement())).await;
        context.sync(operation_id()).unwrap();
        let later = prepare_lifecycle(&mut context, &name, "SELECT 1", Some(statement())).await;
        let later_metadata = context.get_statement(&name).unwrap().unwrap();
        context.postgres_failed(Some(rejected), false).unwrap();
        assert!(Arc::ptr_eq(
            &context.get_statement(&name).unwrap().unwrap(),
            &later_metadata
        ));
        assert!(!context.discarding_extended_messages().unwrap());
        context.postgres_failed(Some(later), false).unwrap();
        assert!(Arc::ptr_eq(
            &context.get_statement(&name).unwrap().unwrap(),
            &original_metadata
        ));
    }

    #[test]
    fn rollback_batch_is_refused_when_it_can_restore_another_keyset() {
        let context = create_context();
        let parse = |sql| crate::postgresql::parser::SqlParser::parse_statement(sql).unwrap();
        {
            let mut state = context.protocol_state.write().unwrap();
            state
                .settings
                .execution_succeeded(&parse("SET CIPHERSTASH.KEYSET_NAME = 'session'"), "SET");
            state.settings.ready_for_query(TransactionStatus::Idle);
            state.settings.execution_succeeded(&parse("BEGIN"), "BEGIN");
            state
                .settings
                .execution_succeeded(&parse("SAVEPOINT before_local"), "SAVEPOINT");
            state
                .settings
                .execution_succeeded(&parse("SET LOCAL CIPHERSTASH.KEYSET_NAME = 'local'"), "SET");
        }
        for sql in [
            "ROLLBACK; SELECT 1",
            "COMMIT; SELECT 1",
            "ROLLBACK TO before_local; SELECT 1",
            "RESET ALL; SELECT 1",
        ] {
            assert!(
                context
                    .settings_simple_query_requires_fail_closed(
                        &crate::postgresql::parser::SqlParser::parse_statements(sql).unwrap()
                    )
                    .unwrap(),
                "{sql}"
            );
        }
    }

    #[tokio::test]
    async fn rejected_parse_removes_its_analysed_metadata() {
        let mut context = create_context();
        let name = Name::from_static(b"named");
        let original = prepare_lifecycle(&mut context, &name, "BEGIN", None).await;
        context.preparation_acknowledged(original).unwrap();
        let rejected = prepare_lifecycle(&mut context, &name, "SELECT 1", Some(statement())).await;
        context.postgres_failed(Some(rejected), false).unwrap();
        assert!(context
            .protocol_state
            .read()
            .unwrap()
            .prepared_statements
            .statement(&name)
            .is_none());
    }

    #[tokio::test]
    async fn successful_discard_all_removes_statement_and_portal_metadata() {
        let mut context = create_context();
        let name = Name::from_static(b"statement");
        let parse = prepare_lifecycle(&mut context, &name, "SELECT 1", Some(statement())).await;
        context.preparation_acknowledged(parse).unwrap();
        let bind = operation_id();
        let knowledge = context.begin_bind(bind, name.clone(), &name).unwrap();
        context
            .finish_bind(bind, Portal::passthrough(knowledge.metrics))
            .unwrap();
        context.preparation_acknowledged(bind).unwrap();
        let discard = operation_id();
        let metrics = context.start_metrics_scope().unwrap();
        context
            .register_query(
                discard,
                metrics,
                Portal::passthrough(Some(metrics)),
                &crate::postgresql::parser::SqlParser::parse_statements("DISCARD ALL").unwrap(),
            )
            .unwrap();
        assert!(context.get_statement(&name).unwrap().is_some());
        assert!(context.get_portal(&name).unwrap().is_some());
        context
            .execution_completed(Some(discard), ExecutionEnd::Command("DISCARD ALL"), false)
            .unwrap();
        assert!(context.get_statement(&name).unwrap().is_none());
        assert!(context
            .test_statement_metrics_scope(&name)
            .unwrap()
            .is_none());
        assert!(context.get_portal(&name).unwrap().is_none());
    }

    #[test]
    fn settings_simple_query_fails_closed_when_protocol_state_is_unavailable() {
        let context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(context
            .settings_simple_query_requires_fail_closed(
                &crate::postgresql::parser::SqlParser::parse_statements("SELECT 1").unwrap()
            )
            .unwrap());
    }

    #[test]
    fn settings_in_flight_when_protocol_state_is_unavailable() {
        let context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.settings_in_flight(),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn plain_transaction_and_reset_batches_are_allowed() {
        let context = create_context();
        for sql in [
            "ROLLBACK; SELECT now()",
            "COMMIT; SELECT 1",
            "RESET ALL; SELECT 1",
        ] {
            let statements = crate::postgresql::parser::SqlParser::parse_statements(sql).unwrap();
            assert!(
                !context
                    .settings_simple_query_requires_fail_closed(&statements)
                    .unwrap(),
                "{sql}"
            );
        }
    }

    #[tokio::test]
    async fn empty_plaintext_batch_does_not_call_encryption_service() {
        let context = create_context();
        let output = context
            .encrypt(vec![None, None], &[None, None])
            .await
            .unwrap();

        assert_eq!(output.len(), 2);
        assert!(output.iter().all(Option::is_none));
    }

    #[tokio::test]
    async fn encryption_service_returning_fewer_values_fails_closed() {
        let context = create_context();
        // TestService returns no outputs at all
        let plaintext = cipherstash_client::encryption::Plaintext::new("secret".to_owned());
        let result = context
            .encrypt(vec![Some(plaintext), None], &[None, None])
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
    fn statement() -> Statement {
        Statement {
            param_columns: vec![],
            projection_columns: vec![],
            literal_columns: vec![],
            postgres_param_types: vec![],
            output_params: vec![],
        }
    }

    fn portal(statement: &Arc<Statement>) -> Portal {
        Portal::encrypted_with_format_codes(statement.clone(), vec![], None)
    }

    #[test]
    fn replacing_a_statement_does_not_finish_an_overlapping_execution_metrics_scope() {
        let mut context = create_context();
        let name = Name::default();
        let session_id = context.start_metrics_scope().unwrap();
        context
            .set_statement_metrics_scope(name.clone(), session_id)
            .unwrap();
        context
            .add_portal(
                operation_id(),
                Name::from("active_portal"),
                Portal::passthrough(Some(session_id)),
            )
            .unwrap();

        context.close_statement(&name).unwrap();

        assert!(context.get_metrics_scope(session_id).unwrap().is_some());
        assert!(context
            .get_statement_metrics_scope(&name)
            .unwrap()
            .is_none());
    }

    #[test]
    fn replacing_a_statement_does_not_finish_a_suspended_execution_scope() {
        let mut context = create_context();
        let statement = Name::from("statement");
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_statement_metrics_scope(statement.clone(), scope)
            .unwrap();
        let operation = operation_id();
        context
            .set_execute(operation, Name::from("portal"), Some(scope))
            .unwrap();
        context
            .finish_execution(operation, ExecutionOutcome::Suspended)
            .unwrap();

        context.close_statement(&statement).unwrap();

        assert!(context.get_metrics_scope(scope).unwrap().is_some());
    }

    #[test]
    fn readiness_releases_reparsed_statement_metrics_after_their_portal_is_destroyed() {
        let mut context = create_context();
        let statement = Name::from("statement");
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_statement_metrics_scope(statement.clone(), scope)
            .unwrap();
        context
            .add_portal(
                operation_id(),
                Name::from("named_portal"),
                Portal::passthrough(Some(scope)),
            )
            .unwrap();

        context.close_statement(&statement).unwrap();
        assert!(context.get_metrics_scope(scope).unwrap().is_some());

        context
            .ready_for_query(TransactionStatus::Idle, Some(operation_id()))
            .unwrap();

        assert!(context.get_metrics_scope(scope).unwrap().is_none());
    }

    #[test]
    fn closing_an_unexecuted_statement_does_not_record_statement_metrics() {
        let mut context = create_context();
        let name = Name::default();
        let session_id = context.start_metrics_scope().unwrap();
        context
            .set_statement_metrics_scope(name.clone(), session_id)
            .unwrap();
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        metrics::with_local_recorder(&recorder, || {
            context.close_statement_explicit(&name).unwrap();
        });

        assert!(context.get_metrics_scope(session_id).unwrap().is_none());
        let rendered = handle.render();
        assert!(
            !rendered.contains("cipherstash_proxy_statements_session_duration_seconds_count"),
            "{rendered}"
        );
    }

    #[test]
    fn replacing_a_statement_retains_existing_portals() {
        let context = create_context();
        let closed_statement_name = Name::from("closed_statement");
        let retained_statement_name = Name::from("retained_statement");
        context
            .add_statement(closed_statement_name.clone(), statement())
            .unwrap();
        context
            .add_statement(retained_statement_name.clone(), statement())
            .unwrap();

        let closed_statement = context
            .get_statement(&closed_statement_name)
            .unwrap()
            .unwrap();
        let retained_statement = context
            .get_statement(&retained_statement_name)
            .unwrap()
            .unwrap();
        let closed_portal_name = Name::from("differently_named_portal");
        let retained_portal_name = Name::from("retained_portal");
        let passthrough_portal_name = Name::from("passthrough_portal");
        context
            .add_portal(
                operation_id(),
                closed_portal_name.clone(),
                portal(&closed_statement),
            )
            .unwrap();
        context
            .add_portal(
                operation_id(),
                retained_portal_name.clone(),
                portal(&retained_statement),
            )
            .unwrap();
        context
            .add_portal(
                operation_id(),
                passthrough_portal_name.clone(),
                Portal::passthrough(None),
            )
            .unwrap();

        context.close_statement(&closed_statement_name).unwrap();

        assert!(context.get_portal(&closed_portal_name).unwrap().is_some());
        assert!(context.get_portal(&retained_portal_name).unwrap().is_some());
        assert!(context
            .get_portal(&passthrough_portal_name)
            .unwrap()
            .is_some());
    }

    #[test]
    fn idle_readiness_finishes_abandoned_suspended_execution_metrics() {
        let mut context = create_context();
        let portal = Name::from("limited_portal");
        let template_scope = context.start_metrics_scope().unwrap();
        context
            .add_portal(
                operation_id(),
                portal.clone(),
                Portal::passthrough(Some(template_scope)),
            )
            .unwrap();
        let operation = operation_id();
        context.set_execute_for_portal(operation, portal).unwrap();
        let execution_scope = context
            .get_execute(operation)
            .unwrap()
            .and_then(|execute| execute.session_id())
            .unwrap();
        context
            .finish_execution(operation, ExecutionOutcome::Suspended)
            .unwrap();

        context
            .ready_for_query(TransactionStatus::Idle, Some(operation_id()))
            .unwrap();

        assert!(context
            .get_metrics_scope(execution_scope)
            .unwrap()
            .is_none());
    }

    #[test]
    fn idle_readiness_finishes_simple_query_execution() {
        let mut context = create_context();
        let operation = operation_id();
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_simple_query_execute_until_ready(operation, Name::new(), Some(scope))
            .unwrap();
        context
            .finish_execution(operation, ExecutionOutcome::Completed)
            .unwrap();
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        metrics::with_local_recorder(&recorder, || {
            context
                .ready_for_query(TransactionStatus::Idle, Some(operation))
                .unwrap();
        });

        assert!(matches!(
            context.get_execute(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
        assert!(context.get_metrics_scope(scope).unwrap().is_none());
        assert!(handle
            .render()
            .contains("cipherstash_proxy_statements_execution_duration_seconds_count"));
    }

    #[test]
    fn readiness_for_one_query_preserves_later_pipelined_work() {
        let mut context = create_context();
        let query_a = operation_id();
        let query_a_scope = context.start_metrics_scope().unwrap();
        context
            .set_simple_query_execute_until_ready(query_a, Name::new(), Some(query_a_scope))
            .unwrap();

        let portal_b = Name::from("pipeline_b");
        let bind_b = operation_id();
        context
            .add_portal(bind_b, portal_b.clone(), Portal::passthrough(None))
            .unwrap();
        let describe_b = operation_id();
        context
            .set_describe(
                describe_b,
                Describe {
                    target: DescribeTarget::Portal,
                    name: portal_b.clone(),
                },
            )
            .unwrap();
        let execute_b = operation_id();
        context
            .set_execute_for_portal(execute_b, portal_b.clone())
            .unwrap();

        context
            .ready_for_query(TransactionStatus::Idle, Some(query_a))
            .unwrap();

        assert!(context.get_metrics_scope(query_a_scope).unwrap().is_none());
        assert!(context.get_portal(&portal_b).unwrap().is_some());
        assert!(context.complete_describe(describe_b).is_ok());
        assert!(context.get_execute(execute_b).unwrap().is_some());
    }

    #[test]
    fn transaction_readiness_finishes_one_query_and_preserves_later_pipelined_work() {
        let mut context = create_context();
        let query_a = operation_id();
        let query_a_scope = context.start_metrics_scope().unwrap();
        context
            .set_simple_query_execute_until_ready(query_a, Name::new(), Some(query_a_scope))
            .unwrap();

        let query_b = operation_id();
        let query_b_scope = context.start_metrics_scope().unwrap();
        context
            .set_simple_query_execute_until_ready(query_b, Name::new(), Some(query_b_scope))
            .unwrap();

        context
            .ready_for_query(TransactionStatus::InTransaction, Some(query_a))
            .unwrap();

        assert!(matches!(
            context.get_execute(query_a),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
        assert!(context.get_metrics_scope(query_a_scope).unwrap().is_none());
        assert!(context.get_execute(query_b).unwrap().is_some());
        assert!(context.get_metrics_scope(query_b_scope).unwrap().is_some());
    }

    #[test]
    fn closing_a_suspended_portal_finishes_its_execution_metrics() {
        let mut context = create_context();
        let portal = Name::from("limited_portal");
        let template_scope = context.start_metrics_scope().unwrap();
        context
            .set_statement_metrics_scope(Name::from("statement"), template_scope)
            .unwrap();
        context
            .add_portal(
                operation_id(),
                portal.clone(),
                Portal::passthrough(Some(template_scope)),
            )
            .unwrap();
        let operation = operation_id();
        context
            .set_execute_for_portal(operation, portal.clone())
            .unwrap();
        let execution_scope = context
            .get_execute(operation)
            .unwrap()
            .and_then(|execute| execute.session_id())
            .unwrap();
        context
            .finish_execution(operation, ExecutionOutcome::Suspended)
            .unwrap();
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        metrics::with_local_recorder(&recorder, || {
            context.close_portal(&portal).unwrap();
        });

        assert!(context
            .get_metrics_scope(execution_scope)
            .unwrap()
            .is_none());
        let rendered = handle.render();
        let count = rendered
            .lines()
            .find(|line| {
                line.starts_with("cipherstash_proxy_statements_session_duration_seconds_count")
            })
            .unwrap();
        assert!(count.ends_with(" 1"), "{rendered}");
    }

    #[test]
    fn rebinding_a_suspended_portal_starts_a_new_execution_occurrence() {
        let mut context = create_context();
        let portal_name = Name::new();
        let first_template = context.start_metrics_scope().unwrap();
        context
            .add_portal(
                operation_id(),
                portal_name.clone(),
                Portal::passthrough(Some(first_template)),
            )
            .unwrap();
        let operation = operation_id();
        context
            .set_execute_for_portal(operation, portal_name.clone())
            .unwrap();
        let first_execution = context
            .get_execute(operation)
            .unwrap()
            .unwrap()
            .session_id()
            .unwrap();
        context
            .finish_execution(operation, ExecutionOutcome::Suspended)
            .unwrap();

        let rebound_template = context.start_metrics_scope().unwrap();
        context
            .add_portal(
                operation_id(),
                portal_name.clone(),
                Portal::passthrough(Some(rebound_template)),
            )
            .unwrap();
        context
            .set_execute_for_portal(operation, portal_name)
            .unwrap();

        assert_eq!(
            context
                .get_portal_from_execute(operation)
                .unwrap()
                .and_then(|portal| portal.session_id()),
            Some(rebound_template)
        );
        assert!(context
            .get_metrics_scope(first_execution)
            .unwrap()
            .is_none());
    }

    #[test]
    fn closing_a_portal_fails_when_protocol_state_is_unavailable() {
        let context = create_context();
        let protocol_state = context.protocol_state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = protocol_state.write().unwrap();
            panic!("poison protocol state");
        })
        .join();

        assert!(matches!(
            context.close_portal(&Name::new()),
            Err(Error::Context(
                crate::error::ContextError::ProtocolStateUnavailable
            ))
        ));
    }

    #[test]
    fn discarding_an_unfinished_operation_releases_its_execution_metrics() {
        let mut context = create_context();
        let portal = Name::from("skipped_portal");
        let template_scope = context.start_metrics_scope().unwrap();
        context
            .add_portal(
                operation_id(),
                portal.clone(),
                Portal::passthrough(Some(template_scope)),
            )
            .unwrap();
        let operation = operation_id();
        context.set_execute_for_portal(operation, portal).unwrap();
        let execution_scope = context
            .get_execute(operation)
            .unwrap()
            .and_then(|execute| execute.session_id())
            .unwrap();

        context.discard_operation(operation).unwrap();

        assert!(matches!(
            context.get_execute(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
        assert!(context
            .get_metrics_scope(execution_scope)
            .unwrap()
            .is_none());
    }

    #[test]
    fn discarding_an_operation_preserves_a_metrics_scope_referenced_by_another_operation() {
        let mut context = create_context();
        let scope = context.start_metrics_scope().unwrap();
        let first_operation = operation_id();
        let second_operation = operation_id();
        context
            .set_execute(first_operation, Name::from("first"), Some(scope))
            .unwrap();
        context
            .set_execute(second_operation, Name::from("second"), Some(scope))
            .unwrap();

        context.discard_operation(first_operation).unwrap();

        assert!(context.get_metrics_scope(scope).unwrap().is_some());

        context.discard_operation(second_operation).unwrap();

        assert!(context.get_metrics_scope(scope).unwrap().is_none());
    }

    #[test]
    fn simple_query_execution_finishes_only_at_readiness() {
        let mut context = create_context();
        let operation = operation_id();
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_simple_query_execute_until_ready(operation, Name::new(), Some(scope))
            .unwrap();

        context
            .finish_execution(operation, ExecutionOutcome::Completed)
            .unwrap();
        assert!(context.get_execute(operation).unwrap().is_some());

        context
            .finish_execution(operation, ExecutionOutcome::Completed)
            .unwrap();
        assert!(context.get_execute(operation).unwrap().is_some());

        context
            .ready_for_query(TransactionStatus::Idle, Some(operation))
            .unwrap();
        assert!(matches!(
            context.get_execute(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
        assert!(context.get_metrics_scope(scope).unwrap().is_none());
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
    pub fn add_and_close_portals() {
        log::init(LogConfig::default());

        let context = create_context();

        // Create multiple statements
        let statement_name_1 = Name::from("statement_1");
        let statement_name_2 = Name::from("statement_2");

        // Add statements to context
        context
            .add_statement(statement_name_1.clone(), statement())
            .unwrap();
        context
            .add_statement(statement_name_2.clone(), statement())
            .unwrap();

        let portal_name = Name::from("portal");

        let statement_1 = context.get_statement(&statement_name_1).unwrap().unwrap();
        context
            .add_portal(operation_id(), portal_name.clone(), portal(&statement_1))
            .unwrap();

        let statement_2 = context.get_statement(&statement_name_2).unwrap().unwrap();
        context
            .add_portal(operation_id(), portal_name.clone(), portal(&statement_2))
            .unwrap();

        let portal = context.get_portal(&portal_name).unwrap().unwrap();
        assert_eq!(statement_2, get_statement(portal));
        context.close_portal(&portal_name).unwrap();
        assert!(context.get_portal(&portal_name).unwrap().is_none());
    }
}
