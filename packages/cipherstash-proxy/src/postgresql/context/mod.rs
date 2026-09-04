pub mod column;
pub mod phase_timing;
pub mod portal;
pub mod statement;
pub mod statement_metadata;
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
use sqltk::parser::ast::{Expr, Ident, ObjectName, ObjectNamePart, Set, Value, ValueWithSpan};
pub use statement_metadata::StatementMetadata;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tracing::{debug, error, warn};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct SessionId(u64);

#[cfg(test)]
pub trait ResultOptionTestExt {
    fn is_some(&self) -> bool;
    fn is_none(&self) -> bool;
}

#[cfg(test)]
impl<T> ResultOptionTestExt for Result<Option<T>, Error> {
    fn is_some(&self) -> bool {
        self.as_ref().unwrap().is_some()
    }

    fn is_none(&self) -> bool {
        match self {
            Ok(value) => value.is_none(),
            Err(Error::Context(crate::error::ContextError::UnknownOperation)) => true,
            Err(err) => panic!("unexpected protocol state error: {err}"),
        }
    }
}

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
    unsafe_disable_mapping: bool,
    keyset_id: Arc<RwLock<Option<KeysetIdentifier>>>,
    session_id_counter: Arc<AtomicU64>,
    transaction_status: Arc<RwLock<TransactionStatus>>,
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
}

#[derive(Clone, Debug)]
struct DescribeContext {
    statement: Option<Arc<Statement>>,
}

#[derive(Debug)]
struct ConnectionProtocolState<K = OperationId> {
    operations: HashMap<K, OperationContext>,
    statement_metrics: HashMap<SessionId, SessionMetricsContext>,
    suspended_executions: HashMap<Name, (K, ExecuteContext)>,
    statements: HashMap<Name, Arc<Statement>>,
    statement_metrics_scopes: HashMap<Name, SessionId>,
    portals: HashMap<Name, Arc<Portal>>,
    portal_operations: HashMap<Name, K>,
}

impl<K> Default for ConnectionProtocolState<K> {
    fn default() -> Self {
        Self {
            operations: HashMap::new(),
            statement_metrics: HashMap::new(),
            suspended_executions: HashMap::new(),
            statements: HashMap::new(),
            statement_metrics_scopes: HashMap::new(),
            portals: HashMap::new(),
            portal_operations: HashMap::new(),
        }
    }
}

impl<K> ConnectionProtocolState<K>
where
    K: Copy + Eq + std::hash::Hash,
{
    fn metrics_referenced(&self, session_id: SessionId) -> bool {
        self.statement_metrics_scopes
            .values()
            .any(|id| *id == session_id)
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
    records_session: bool,
    pub phase_timing: PhaseTiming,
    pub metadata: StatementMetadata,
}

impl SessionMetricsContext {
    fn new(id: SessionId) -> SessionMetricsContext {
        SessionMetricsContext {
            id,
            start: Instant::now(),
            records_session: true,
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
            unsafe_disable_mapping: false,
            keyset_id: Arc::new(RwLock::new(None)),
            session_id_counter: Arc::new(AtomicU64::new(1)),
            transaction_status: Arc::new(RwLock::new(TransactionStatus::Idle)),
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
            } => state.statements.get(name).cloned(),
        };
        state.operations.entry(operation).or_default().describe =
            Some(DescribeContext { statement });
        Ok(())
    }

    pub fn set_non_execution(&self, operation: OperationId) -> Result<(), Error> {
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .operations
            .entry(operation)
            .or_default();
        Ok(())
    }

    pub fn complete_non_execution(&self, operation: OperationId) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        state.operations.remove(&operation);
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
        let session = session_id.and_then(|id| state.statement_metrics.remove(&id));
        drop(state);
        self.record_finished_session(session);
        Ok(())
    }

    fn record_finished_session(&self, session: Option<SessionMetricsContext>) {
        if let Some(session) = session {
            if !session.records_session {
                return;
            }
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

    pub fn set_execute(
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

    pub fn set_simple_query_execute_until_ready(
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

    /// Set execute state for portal, looking up session ID internally.
    pub fn set_execute_for_portal(
        &mut self,
        operation: OperationId,
        name: Name,
    ) -> Result<(), Error> {
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
                metrics.records_session = true;
                state.statement_metrics.insert(execution_id, metrics);
                ExecuteContext::new(name, portal, Some(execution_id))
            });
        state.operations.entry(operation).or_default().execute = Some(execute);
        Ok(())
    }

    /// Applies one terminal or suspended Execute outcome atomically.
    pub fn finish_execution(
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
        self.record_finished_session(transition.finished_metrics);
        Ok(transition.replacement_error)
    }

    fn record_execution_duration(
        &self,
        execute: &ExecuteContext,
        metrics: Option<&SessionMetricsContext>,
    ) {
        let (statement_type, protocol, mapped, multi_statement) = metrics
            .map(|session| {
                let metadata = &session.metadata;
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

    pub fn add_statement(&self, name: Name, statement: Statement) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);
        self.protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?
            .statements
            .insert(name, Arc::new(statement));
        Ok(())
    }

    pub fn close_statement(&self, name: &Name) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);

        let finished_session = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            let session_id = state.statement_metrics_scopes.remove(name);
            state.statements.remove(name);
            let session_in_use = session_id.is_some_and(|id| state.metrics_referenced(id));
            if !session_in_use {
                session_id.and_then(|session_id| state.statement_metrics.remove(&session_id))
            } else {
                None
            }
        };
        self.record_finished_session(finished_session);
        Ok(())
    }

    pub fn transaction_status(&self) -> TransactionStatus {
        self.transaction_status
            .read()
            .map(|status| *status)
            .unwrap_or(TransactionStatus::Idle)
    }

    pub fn ready_for_query(
        &self,
        status: TransactionStatus,
        boundary: Option<OperationId>,
    ) -> Result<(), Error> {
        *self
            .transaction_status
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)? = status;
        let Some(boundary) = boundary else {
            return Ok(());
        };

        let (finished_executions, finished_sessions) = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            let mut candidates = Vec::new();
            let mut executions = Vec::new();
            if status == TransactionStatus::Idle {
                let portal_names = state
                    .portal_operations
                    .iter()
                    .filter_map(|(name, operation)| {
                        (*operation <= boundary).then_some(name.clone())
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
        for session in finished_sessions {
            self.record_finished_session(Some(session));
        }
        Ok(())
    }

    /// Close a statement explicitly requested by the client.
    ///
    /// PostgreSQL portals retain the parsed statement they reference and remain
    /// valid after the statement name is closed, so they must not be removed.
    pub fn close_statement_explicit(&self, name: &Name) -> Result<(), Error> {
        self.close_statement(name)
    }

    pub fn discard_operation(&mut self, operation: OperationId) -> Result<(), Error> {
        let finished_session = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            let session_id = state
                .operations
                .remove(&operation)
                .and_then(|operation| operation.execute)
                .and_then(|execute| execute.session_id());
            session_id.and_then(|session_id| state.statement_metrics.remove(&session_id))
        };
        self.record_finished_session(finished_session);
        Ok(())
    }

    pub fn set_operation_error(
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

    pub fn add_portal(
        &self,
        operation: OperationId,
        name: Name,
        portal: Portal,
    ) -> Result<(), Error> {
        debug!(target: CONTEXT, client_id = self.client_id, name = ?name, portal = ?portal);
        let finished_metrics = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
            let mut candidates = Vec::new();
            candidates.extend(
                state
                    .portals
                    .remove(&name)
                    .and_then(|portal| portal.session_id()),
            );
            candidates.extend(
                state
                    .suspended_executions
                    .remove(&name)
                    .and_then(|(_, execute)| execute.session_id()),
            );
            state.portal_operations.insert(name.clone(), operation);
            state.portals.insert(name, Arc::new(portal));
            state.take_unreferenced_metrics(candidates)
        };
        for metrics in finished_metrics {
            self.record_finished_session(Some(metrics));
        }
        Ok(())
    }

    pub fn get_statement(&self, name: &Name) -> Result<Option<Arc<Statement>>, Error> {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state.statements.get(name).cloned())
    }

    pub fn set_statement_session(
        &mut self,
        name: Name,
        session_id: SessionId,
    ) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        if let Some(metrics) = state.statement_metrics.get_mut(&session_id) {
            metrics.records_session = false;
        }
        state.statement_metrics_scopes.insert(name, session_id);
        Ok(())
    }

    pub fn get_statement_session(&self, name: &Name) -> Result<Option<SessionId>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        Ok(state.statement_metrics_scopes.get(name).copied())
    }

    pub fn start_portal_metrics_scope(
        &mut self,
        statement: &Name,
    ) -> Result<Option<SessionId>, Error> {
        let id = SessionId(self.session_id_counter.fetch_add(1, Ordering::Relaxed));
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let Some(template_id) = state.statement_metrics_scopes.get(statement).copied() else {
            return Ok(None);
        };
        let Some(mut metrics) = state.statement_metrics.get(&template_id).cloned() else {
            return Err(crate::error::ContextError::StatementMetricsUnavailable.into());
        };
        metrics.id = id;
        metrics.start = Instant::now();
        metrics.records_session = false;
        state.statement_metrics.insert(id, metrics);
        Ok(Some(id))
    }

    ///
    /// Close the portal identified by `name`
    /// Portal is removed from  queue
    ///
    pub fn close_portal(&self, name: &Name) -> Result<(), Error> {
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
            self.record_finished_session(Some(metrics));
        }
        Ok(())
    }

    fn close_portal_if_current(
        &self,
        name: &Name,
        expected: Option<&Arc<Portal>>,
    ) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        if expected.is_some_and(|expected| {
            state
                .portals
                .get(name)
                .is_some_and(|current| Arc::ptr_eq(current, expected))
        }) {
            state.portals.remove(name);
            state.portal_operations.remove(name);
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

    pub fn get_portal_session_id(&self, name: &Name) -> Result<Option<SessionId>, Error> {
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

    pub fn get_session_metrics(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionMetricsContext>, Error> {
        let state = self
            .protocol_state
            .read()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        let Some(session_context) = state.statement_metrics.get(&session_id) else {
            return Ok(None);
        };
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Get Session Metrics", session_metrics = ?session_context);
        Ok(Some(session_context.to_owned()))
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
    pub fn prepare_schema_statement(&self, name: Name, statement: sqltk::parser::ast::Statement) {
        self.schema_middleware.prepare(name, statement);
    }

    /// Associates a bound portal with its prepared statement's schema intent.
    pub fn bind_schema_statement(&self, portal: Name, prepared_statement: &Name) {
        self.schema_middleware.bind(portal, prepared_statement);
    }

    /// Records a portal execution and returns whether its DDL needs an injected flush.
    pub fn execute_schema_portal(&self, portal: &Name) -> bool {
        self.schema_middleware.execute(portal)
    }

    /// Records statements in one simple-query protocol message.
    pub fn execute_simple_schema_statements(&self, statements: &[sqltk::parser::ast::Statement]) {
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
    pub fn mark_schema_protocol_boundary(&self) {
        self.schema_middleware.protocol_boundary();
    }

    /// Waits for preceding schema-changing executions to resolve.
    pub async fn wait_for_schema_execution(&self) {
        self.schema_middleware.wait_for_ddl().await;
    }

    pub fn report_schema_execution_succeeded(&self) {
        self.schema_middleware.execution_succeeded();
    }

    pub fn report_schema_execution_failed(&self) {
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
    pub fn schema_ready_for_query(&self, status: crate::proxy::schema::TransactionStatus) {
        self.schema_middleware.ready_for_query(status);
    }

    /// Examines a [`sqltk::parser::ast::Statement`] and if it is precisely equal to `SET UNSAFE_DISABLE_MAPPING = {boolean};`
    /// then it sets the flag [`Context::unsafe_disable_mapping`] to the provided `{boolean}`` value.
    ///
    ///
    pub fn maybe_set_unsafe_disable_mapping(
        &mut self,
        statement: &sqltk::parser::ast::Statement,
    ) -> Option<bool> {
        // The CIPHERSTASH. namespace prevents errors UNSAFE_DISABLE_MAPPING
        // The constants avoid the need to allocate Vecs every time we examine the statement.
        static SQL_SETTING_NAME_UNSAFE_DISABLE_MAPPING: LazyLock<ObjectName> =
            LazyLock::new(|| {
                ObjectName(vec![
                    ObjectNamePart::Identifier(Ident::new("CIPHERSTASH")),
                    ObjectNamePart::Identifier(Ident::new("UNSAFE_DISABLE_MAPPING")),
                ])
            });

        if let sqltk::parser::ast::Statement::Set(Set::SingleAssignment {
            variable, values, ..
        }) = statement
        {
            if variable == &*SQL_SETTING_NAME_UNSAFE_DISABLE_MAPPING {
                if let Some(Expr::Value(ValueWithSpan {
                    value: Value::Boolean(value),
                    ..
                })) = values.first()
                {
                    self.unsafe_disable_mapping = *value;
                    return Some(*value);
                }
            }
        }
        None
    }

    pub fn unsafe_disable_mapping(&mut self) -> bool {
        self.unsafe_disable_mapping
    }

    /// Examines a [`sqltk::parser::ast::Statement`] and if it is precisely equal to `SET CIPHERSTASH.KEYSET_ID = {keyset_id};`
    /// then it sets the [`Context::keyset_id`] to the provided `{keyset_id}`` value.
    ///
    ///
    pub fn maybe_set_keyset_id(
        &mut self,
        statement: &sqltk::parser::ast::Statement,
    ) -> Result<Option<KeysetIdentifier>, Error> {
        // The CIPHERSTASH. namespace prevents errors KEYSET_ID
        // The constants avoid the need to allocate Vecs every time we examine the statement.
        static SQL_SETTING_NAME_KEYSET_ID: LazyLock<ObjectName> = LazyLock::new(|| {
            ObjectName(vec![
                ObjectNamePart::Identifier(Ident::new("CIPHERSTASH")),
                ObjectNamePart::Identifier(Ident::new("KEYSET_ID")),
            ])
        });

        if let sqltk::parser::ast::Statement::Set(Set::SingleAssignment {
            variable, values, ..
        }) = statement
        {
            if variable == &*SQL_SETTING_NAME_KEYSET_ID {
                if let Some(Expr::Value(ValueWithSpan { value, .. })) = values.first() {
                    let value_str = match value {
                        Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => s.clone(),
                        Value::Number(n, _) => n.to_string(),
                        _ => {
                            let err = EncryptError::KeysetIdCouldNotBeSet;
                            warn!(target: CONTEXT, client_id = self.client_id, msg = err.to_string());
                            return Ok(None);
                        }
                    };
                    let keyset_id = Uuid::parse_str(&value_str).map_err(|_| {
                        EncryptError::KeysetIdCouldNotBeParsed {
                            id: value_str.to_owned(),
                        }
                    })?;

                    debug!(target: CONTEXT, client_id = self.client_id, msg = "Set KeysetId", ?keyset_id);

                    let identifier = KeysetIdentifier(IdentifiedBy::Uuid(keyset_id));
                    let _ = self
                        .keyset_id
                        .write()
                        .map(|mut guard| *guard = Some(identifier.clone()));

                    return Ok(Some(identifier));
                } else {
                    let err = EncryptError::KeysetIdCouldNotBeSet;
                    warn!(target: CONTEXT, client_id = self.client_id, msg = err.to_string());
                    // We let the database handle any syntax errors to avoid complexifying the fronted flow (more)
                }
            }
        }
        Ok(None)
    }

    /// Examines a [`sqltk::parser::ast::Statement`] and if it is precisely equal to `SET CIPHERSTASH.KEYSET_NAME = {keyset_name};`
    /// then it sets the [`Context::keyset_id`] to the provided `{keyset_name}`` value.
    ///
    ///
    pub fn maybe_set_keyset_name(
        &mut self,
        statement: &sqltk::parser::ast::Statement,
    ) -> Result<Option<KeysetIdentifier>, Error> {
        // The CIPHERSTASH. namespace prevents errors KEYSET_NAME
        // The constants avoid the need to allocate Vecs every time we examine the statement.
        static SQL_SETTING_NAME_KEYSET_NAME: LazyLock<ObjectName> = LazyLock::new(|| {
            ObjectName(vec![
                ObjectNamePart::Identifier(Ident::new("CIPHERSTASH")),
                ObjectNamePart::Identifier(Ident::new("KEYSET_NAME")),
            ])
        });

        if let sqltk::parser::ast::Statement::Set(Set::SingleAssignment {
            variable, values, ..
        }) = statement
        {
            if variable == &*SQL_SETTING_NAME_KEYSET_NAME {
                // Try to extract keyset name from Value (quoted string/number) or Identifier (unquoted)
                let keyset_name = match values.first() {
                    Some(Expr::Value(ValueWithSpan { value, .. })) => match value {
                        Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => {
                            Some(s.clone())
                        }
                        Value::Number(n, _) => Some(n.to_string()),
                        _ => None,
                    },
                    Some(Expr::Identifier(ident)) => Some(ident.value.clone()),
                    _ => None,
                };

                if let Some(keyset_name) = keyset_name {
                    debug!(target: CONTEXT, client_id = self.client_id, msg = "Set KeysetName", ?keyset_name);

                    let identifier = KeysetIdentifier(IdentifiedBy::Name(keyset_name.into()));
                    let _ = self
                        .keyset_id
                        .write()
                        .map(|mut guard| *guard = Some(identifier.clone()));

                    return Ok(Some(identifier));
                } else {
                    let err = EncryptError::KeysetNameCouldNotBeSet;
                    warn!(target: CONTEXT, client_id = self.client_id, msg = err.to_string());
                    // We let the database handle any syntax errors to avoid complexifying the frontend flow
                }
            }
        }
        Ok(None)
    }

    /// Single entry point for setting keyset identifiers by either ID or name.
    /// Tries to set keyset_id first, then keyset_name if that doesn't match.
    ///
    pub fn maybe_set_keyset(
        &mut self,
        statement: &sqltk::parser::ast::Statement,
    ) -> Result<Option<KeysetIdentifier>, Error> {
        match self.maybe_set_keyset_id(statement)? {
            Some(identifier) => Ok(Some(identifier)),
            None => self.maybe_set_keyset_name(statement),
        }
    }

    pub fn keyset_identifier(&self) -> Option<KeysetIdentifier> {
        self.keyset_id.read().ok().and_then(|k| k.clone())
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

        let keyset_id = self.keyset_identifier();

        self.encryption
            .encrypt(keyset_id, plaintexts, columns)
            .await
    }

    pub async fn decrypt(
        &self,
        ciphertexts: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
        let keyset_id = self.keyset_identifier();
        self.encryption.decrypt(keyset_id, ciphertexts).await
    }

    pub async fn decrypt_inbound_eql(
        &self,
        ciphertexts: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
        let keyset_id = self.keyset_identifier();
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

    fn with_session_metrics_mut<F>(&mut self, session_id: SessionId, f: F) -> Result<(), Error>
    where
        F: FnOnce(&mut SessionMetricsContext),
    {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| crate::error::ContextError::ProtocolStateUnavailable)?;
        if let Some(session) = state.statement_metrics.get_mut(&session_id) {
            f(session);
        }
        Ok(())
    }

    /// Record parse phase duration for the session (first write wins)
    pub fn record_parse_duration(
        &mut self,
        session_id: SessionId,
        duration: Duration,
    ) -> Result<(), Error> {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.record_parse(duration);
        })
    }

    /// Add encrypt phase duration for the session (accumulate)
    pub fn add_encrypt_duration(
        &mut self,
        session_id: SessionId,
        duration: Duration,
    ) -> Result<(), Error> {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.add_encrypt(duration);
        })
    }

    /// Add decrypt phase duration (accumulate)
    pub fn add_decrypt_duration(
        &mut self,
        session_id: SessionId,
        duration: Duration,
    ) -> Result<(), Error> {
        self.with_session_metrics_mut(session_id, |session| {
            session.phase_timing.add_decrypt(duration);
        })
    }

    /// Update statement metadata for a session
    pub fn update_statement_metadata<F>(&mut self, session_id: SessionId, f: F) -> Result<(), Error>
    where
        F: FnOnce(&mut StatementMetadata),
    {
        self.with_session_metrics_mut(session_id, |session| {
            f(&mut session.metadata);
        })
    }

    /// Update statement metadata if session ID is present, no-op otherwise.
    pub fn with_session<F>(&mut self, session_id: Option<SessionId>, f: F) -> Result<(), Error>
    where
        F: FnOnce(&mut SessionMetricsContext),
    {
        if let Some(sid) = session_id {
            self.with_session_metrics_mut(sid, f)?;
        }
        Ok(())
    }

    /// Add decrypt phase duration for the current execute session (if any)
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

#[cfg(test)]
mod tests {
    use super::{
        ConnectionProtocolState, Context, ExecuteContext, ExecutionOutcome, KeysetIdentifier,
        OperationContext, Portal, ResultOptionTestExt as _, SessionId, SessionMetricsContext,
        Statement,
    };
    use crate::{
        config::LogConfig,
        error::Error,
        log,
        postgresql::{rewrite::Name, test_operation_id as operation_id, Column},
        proxy::{EncryptConfig, EncryptionService},
        TandemConfig,
    };
    use cipherstash_client::IdentifiedBy;
    use eql_mapper::Schema;
    use pg_proto::{Describe, DescribeTarget, TransactionStatus};
    use sqltk::parser::{dialect::PostgreSqlDialect, parser::Parser};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use uuid::Uuid;

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
            .set_statement_session(statement, template_scope)
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
                .get_session_metrics(execution_scope)
                .unwrap()
                .unwrap()
                .records_session
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
            context.set_statement_session(Name::new(), SessionId(1)),
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

        assert!(context.get_execute(operation).is_some());
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
    fn replacing_a_statement_does_not_finish_an_overlapping_execution_session() {
        let mut context = create_context();
        let name = Name::default();
        let session_id = context.start_metrics_scope().unwrap();
        context
            .set_statement_session(name.clone(), session_id)
            .unwrap();
        context
            .add_portal(
                operation_id(),
                Name::from("active_portal"),
                Portal::passthrough(Some(session_id)),
            )
            .unwrap();

        context.close_statement(&name).unwrap();

        assert!(context.get_session_metrics(session_id).is_some());
        assert!(context.get_statement_session(&name).is_none());
    }

    #[test]
    fn replacing_a_statement_does_not_finish_a_suspended_execution_scope() {
        let mut context = create_context();
        let statement = Name::from("statement");
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_statement_session(statement.clone(), scope)
            .unwrap();
        let operation = operation_id();
        context
            .set_execute(operation, Name::from("portal"), Some(scope))
            .unwrap();
        context
            .finish_execution(operation, ExecutionOutcome::Suspended)
            .unwrap();

        context.close_statement(&statement).unwrap();

        assert!(context.get_session_metrics(scope).is_some());
    }

    #[test]
    fn readiness_releases_reparsed_statement_metrics_after_their_portal_is_destroyed() {
        let mut context = create_context();
        let statement = Name::from("statement");
        let scope = context.start_metrics_scope().unwrap();
        context
            .set_statement_session(statement.clone(), scope)
            .unwrap();
        context
            .add_portal(
                operation_id(),
                Name::from("named_portal"),
                Portal::passthrough(Some(scope)),
            )
            .unwrap();

        context.close_statement(&statement).unwrap();
        assert!(context.get_session_metrics(scope).is_some());

        context
            .ready_for_query(TransactionStatus::Idle, Some(operation_id()))
            .unwrap();

        assert!(context.get_session_metrics(scope).is_none());
    }

    #[test]
    fn closing_an_unexecuted_statement_does_not_record_session_metrics() {
        let mut context = create_context();
        let name = Name::default();
        let session_id = context.start_metrics_scope().unwrap();
        context
            .set_statement_session(name.clone(), session_id)
            .unwrap();
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        metrics::with_local_recorder(&recorder, || {
            context.close_statement_explicit(&name).unwrap();
        });

        assert!(context.get_session_metrics(session_id).is_none());
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

        assert!(context.get_portal(&closed_portal_name).is_some());
        assert!(context.get_portal(&retained_portal_name).is_some());
        assert!(context.get_portal(&passthrough_portal_name).is_some());
    }

    #[test]
    fn transaction_status_tracks_backend_ready_state() {
        let context = create_context();
        assert_eq!(context.transaction_status(), TransactionStatus::Idle);

        context
            .ready_for_query(TransactionStatus::InTransaction, None)
            .unwrap();

        assert_eq!(
            context.transaction_status(),
            TransactionStatus::InTransaction
        );
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

        assert!(context.get_session_metrics(execution_scope).is_none());
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

        assert!(context.get_execute(operation).is_none());
        assert!(context.get_session_metrics(scope).is_none());
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

        assert!(context.get_session_metrics(query_a_scope).is_none());
        assert!(context.get_portal(&portal_b).is_some());
        assert!(context.complete_describe(describe_b).is_ok());
        assert!(context.get_execute(execute_b).is_some());
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

        assert!(context.get_execute(query_a).is_none());
        assert!(context.get_session_metrics(query_a_scope).is_none());
        assert!(context.get_execute(query_b).is_some());
        assert!(context.get_session_metrics(query_b_scope).is_some());
    }

    #[test]
    fn closing_a_suspended_portal_finishes_its_execution_metrics() {
        let mut context = create_context();
        let portal = Name::from("limited_portal");
        let template_scope = context.start_metrics_scope().unwrap();
        context
            .set_statement_session(Name::from("statement"), template_scope)
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

        assert!(context.get_session_metrics(execution_scope).is_none());
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
        assert!(context.get_session_metrics(first_execution).is_none());
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

        assert!(context.get_execute(operation).is_none());
        assert!(context.get_session_metrics(execution_scope).is_none());
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
        assert!(context.get_execute(operation).is_some());

        context
            .finish_execution(operation, ExecutionOutcome::Completed)
            .unwrap();
        assert!(context.get_execute(operation).is_some());

        context
            .ready_for_query(TransactionStatus::Idle, Some(operation))
            .unwrap();
        assert!(context.get_execute(operation).is_none());
        assert!(context.get_session_metrics(scope).is_none());
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
        assert!(context.get_portal(&portal_name).is_none());
    }

    fn parse_statement(sql: &str) -> sqltk::parser::ast::Statement {
        let statements = Parser::new(&PostgreSqlDialect {})
            .try_with_sql(sql)
            .unwrap()
            .parse_statements()
            .unwrap();

        statements.first().unwrap().clone()
    }

    #[test]
    pub fn disable_mapping() {
        log::init(LogConfig::default());

        let mut context = create_context();

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
        let statement = parse_statement(sql);

        context.maybe_set_unsafe_disable_mapping(&statement);
        assert!(context.unsafe_disable_mapping());

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false";
        let statement = parse_statement(sql);

        context.maybe_set_unsafe_disable_mapping(&statement);
        assert!(!context.unsafe_disable_mapping());

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = 1";
        let statement = parse_statement(sql);

        context.maybe_set_unsafe_disable_mapping(&statement);
        assert!(!context.unsafe_disable_mapping());

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = '1'";
        let statement = parse_statement(sql);

        context.maybe_set_unsafe_disable_mapping(&statement);
        assert!(!context.unsafe_disable_mapping());

        let sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = t";
        let statement = parse_statement(sql);

        context.maybe_set_unsafe_disable_mapping(&statement);
        assert!(!context.unsafe_disable_mapping());
    }

    #[test]
    pub fn set_keyset_id() {
        log::init(LogConfig::default());

        let uuid = Uuid::parse_str("7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590").unwrap();

        let identifier = KeysetIdentifier(IdentifiedBy::Uuid(uuid));

        let sql = vec![
            "SET CIPHERSTASH.KEYSET_ID = '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'",
            "SET SESSION CIPHERSTASH.KEYSET_ID = '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'",
            "SET CIPHERSTASH.KEYSET_ID TO '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'",
        ];

        for s in sql {
            let mut context = create_context();
            assert!(context.keyset_identifier().is_none());

            let statement = parse_statement(s);
            let result = context.maybe_set_keyset_id(&statement);

            // OK and has a value
            assert!(result.is_ok());
            assert!(result.unwrap().is_some());

            // keyset id set
            assert_eq!(Some(identifier.clone()), context.keyset_identifier());
        }
    }

    #[test]
    pub fn set_keyset_id_error_handling() {
        log::init(LogConfig::default());

        let mut context = create_context();

        // Returns OK if unknown command
        let sql = "SET CIPHERSTASH.BLAH = 'keyset_id'";
        let statement = parse_statement(sql);

        let result = context.maybe_set_keyset_id(&statement);
        assert!(result.is_ok());

        // Value is NONE as nothing was set
        let value = result.unwrap();
        assert!(value.is_none());

        // Returns OK(None) if SET but badly formatted (no quotes)
        let sql = "SET CIPHERSTASH.KEYSET_ID = d74cbd7fba0d49859ed2ebe2ffe77590";
        let statement = parse_statement(sql);

        let result = context.maybe_set_keyset_id(&statement);

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());

        // Returns ERROR if SET but not UUIOD
        let sql = "SET CIPHERSTASH.KEYSET_ID = 'keyset_id'";
        let statement = parse_statement(sql);

        let result = context.maybe_set_keyset_id(&statement);

        assert!(result.is_err());
    }

    #[test]
    pub fn set_keyset_name() {
        log::init(LogConfig::default());

        let sql = vec![
            "SET CIPHERSTASH.KEYSET_NAME = 'test-keyset'",
            "SET SESSION CIPHERSTASH.KEYSET_NAME = 'test-keyset'",
            "SET CIPHERSTASH.KEYSET_NAME TO 'test-keyset'",
        ];

        for s in sql {
            let mut context = create_context();
            assert!(context.keyset_identifier().is_none());

            let statement = parse_statement(s);
            let result = context.maybe_set_keyset_name(&statement);

            let identifier = KeysetIdentifier(IdentifiedBy::Name("test-keyset".to_string().into()));

            // OK and has a value
            assert!(result.is_ok());
            assert!(result.unwrap().is_some());

            assert_eq!(Some(identifier.clone()), context.keyset_identifier());
        }
    }

    #[test]
    pub fn set_keyset_name_error_handling() {
        log::init(LogConfig::default());

        let mut context = create_context();

        // Returns OK if unknown command
        let sql = "SET CIPHERSTASH.BLAH = 'keyset_name'";
        let statement = parse_statement(sql);

        let result = context.maybe_set_keyset_name(&statement);
        assert!(result.is_ok());

        // Value is NONE as nothing was set
        let value = result.unwrap();
        assert!(value.is_none());

        // Returns OK(None) if SET but badly formatted (unquoted)
        let sql = "SET CIPHERSTASH.KEYSET_NAME = test-keyset";
        let statement = parse_statement(sql);

        let result = context.maybe_set_keyset_name(&statement);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());

        // Returns OK(Some) if SET with number value (now supported)
        let sql = "SET CIPHERSTASH.KEYSET_NAME = 123";
        let statement = parse_statement(sql);

        let identifier = KeysetIdentifier(IdentifiedBy::Name("123".to_string().into()));
        let result = context.maybe_set_keyset_name(&statement);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
        assert_eq!(Some(identifier.clone()), context.keyset_identifier());
    }

    #[test]
    pub fn set_keyset_supports_numbers() {
        log::init(LogConfig::default());

        // Test keyset name with number
        let mut context = create_context();
        let sql = "SET CIPHERSTASH.KEYSET_NAME = 12345";
        let statement = parse_statement(sql);

        let identifier = KeysetIdentifier(IdentifiedBy::Name("12345".to_string().into()));
        let result = context.maybe_set_keyset_name(&statement);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
        assert_eq!(Some(identifier.clone()), context.keyset_identifier());

        // Test keyset id with numeric UUID (should work if it's a valid UUID)
        let mut context = create_context();
        // This will fail because 123 is not a valid UUID, but it shows the number is processed
        let sql = "SET CIPHERSTASH.KEYSET_ID = 123";
        let statement = parse_statement(sql);
        let result = context.maybe_set_keyset_id(&statement);

        // Should return error because 123 is not a valid UUID
        assert!(result.is_err());
    }

    #[test]
    pub fn maybe_set_keyset_unified_function() {
        log::init(LogConfig::default());

        // Test that maybe_set_keyset handles both ID and name
        let mut context = create_context();

        // Test with keyset ID
        let keyset_id_sql = "SET CIPHERSTASH.KEYSET_ID = '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'";
        let statement = parse_statement(keyset_id_sql);

        let uuid = Uuid::parse_str("7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590").unwrap();

        let identifier = KeysetIdentifier(IdentifiedBy::Uuid(uuid));
        let result = context.maybe_set_keyset(&statement);

        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
        assert_eq!(Some(identifier.clone()), context.keyset_identifier());

        // Test with keyset name
        let mut context = create_context();
        let keyset_name_sql = "SET CIPHERSTASH.KEYSET_NAME = 'test-keyset'";
        let statement = parse_statement(keyset_name_sql);

        let identifier = KeysetIdentifier(IdentifiedBy::Name("test-keyset".to_string().into()));
        let result = context.maybe_set_keyset(&statement);

        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
        assert_eq!(Some(identifier.clone()), context.keyset_identifier());

        // Test with unknown command
        let mut context = create_context();
        let unknown_sql = "SET CIPHERSTASH.UNKNOWN = 'value'";
        let statement = parse_statement(unknown_sql);
        let result = context.maybe_set_keyset(&statement);

        assert!(result.is_ok());
        let identifier = result.unwrap();
        assert!(identifier.is_none());
    }
}
