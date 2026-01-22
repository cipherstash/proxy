pub mod column;
pub mod phase_timing;
pub mod portal;
pub mod statement;
pub mod statement_metadata;
pub use self::{phase_timing::{PhaseTiming, PhaseTimer}, portal::Portal, statement::Statement};
pub use statement_metadata::{StatementMetadata, StatementType, ProtocolType};
use super::{
    column_mapper::ColumnMapper,
    messages::{describe::Describe, Name, Target},
    Column,
};
use crate::{
    config::TandemConfig,
    error::{EncryptError, Error},
    log::{CONTEXT, SLOW_STATEMENTS},
    prometheus::{STATEMENTS_EXECUTION_DURATION_SECONDS, STATEMENTS_SESSION_DURATION_SECONDS, SLOW_STATEMENTS_TOTAL},
    proxy::{EncryptConfig, EncryptionService, ReloadCommand, ReloadSender},
};
use cipherstash_client::IdentifiedBy;
use eql_mapper::{Schema, TableResolver};
use metrics::{counter, histogram};
use serde_json::json;
use sqltk::parser::ast::{Expr, Ident, ObjectName, ObjectNamePart, Set, Value, ValueWithSpan};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tracing::{debug, error, warn};
use uuid::Uuid;

type DescribeQueue = Queue<Describe>;
type ExecuteQueue = Queue<ExecuteContext>;
type SessionMetricsQueue = Queue<SessionMetricsContext>;
type PortalQueue = Queue<Arc<Portal>>;

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
    portals: Arc<RwLock<HashMap<Name, PortalQueue>>>,
    describe: Arc<RwLock<DescribeQueue>>,
    execute: Arc<RwLock<ExecuteQueue>>,
    schema_changed: Arc<RwLock<bool>>,
    session_metrics: Arc<RwLock<SessionMetricsQueue>>,
    table_resolver: Arc<TableResolver>,
    unsafe_disable_mapping: bool,
    keyset_id: Arc<RwLock<Option<KeysetIdentifier>>>,
}

#[derive(Clone, Debug)]
pub struct ExecuteContext {
    name: Name,
    start: Instant,
}

impl ExecuteContext {
    fn new(name: Name) -> ExecuteContext {
        ExecuteContext {
            name,
            start: Instant::now(),
        }
    }

    fn duration(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }
}

#[derive(Clone, Debug)]
pub struct SessionMetricsContext {
    start: Instant,
    pub phase_timing: PhaseTiming,
    pub metadata: StatementMetadata,
}

impl SessionMetricsContext {
    fn new() -> SessionMetricsContext {
        SessionMetricsContext {
            start: Instant::now(),
            phase_timing: PhaseTiming::new(),
            metadata: StatementMetadata::new(),
        }
    }

    fn duration(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }
}

#[derive(Clone, Debug)]
pub struct Queue<T> {
    pub queue: VecDeque<T>,
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
            unsafe_disable_mapping: false,
            keyset_id: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_describe(&mut self, describe: Describe) {
        debug!(target: CONTEXT, client_id = self.client_id, describe = ?describe);
        let _ = self.describe.write().map(|mut queue| queue.add(describe));
    }
    ///
    /// Marks the current Describe as complete
    /// Removes the Describe from the Queue
    ///
    pub fn complete_describe(&mut self) {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Describe complete");
        let _ = self.describe.write().map(|mut queue| queue.complete());
    }

    pub fn start_session(&mut self) {
        let ctx = SessionMetricsContext::new();
        let _ = self.session_metrics.write().map(|mut queue| queue.add(ctx));
    }

    pub fn finish_session(&mut self) {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Session Metrics finished");

        if let Some(session) = self.get_session_metrics() {
            let duration = session.duration();
            let metadata = &session.metadata;

            // Get labels for metrics
            let statement_type = metadata.statement_type
                .map(|t| t.as_label())
                .unwrap_or("unknown");
            let protocol = metadata.protocol
                .map(|p| p.as_label())
                .unwrap_or("unknown");
            let mapped = if metadata.encrypted { "true" } else { "false" };
            let multi_statement = if metadata.multi_statement { "true" } else { "false" };

            // Record with labels
            histogram!(
                STATEMENTS_SESSION_DURATION_SECONDS,
                "statement_type" => statement_type,
                "protocol" => protocol,
                "mapped" => mapped,
                "multi_statement" => multi_statement
            ).record(duration);

            // Log slow statements when enabled
            if self.config.slow_statements_enabled() && duration > self.config.slow_statement_min_duration() {
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

    pub fn set_execute(&mut self, name: Name) {
        debug!(target: CONTEXT, client_id = self.client_id, execute = ?name);

        let ctx = ExecuteContext::new(name);

        let _ = self.execute.write().map(|mut queue| queue.add(ctx));
    }

    ///
    /// Marks the current Execution as Complete
    ///
    /// If the associated portal is Unnamed, it is closed
    ///
    /// From the PostgreSQL Extended Query docs:
    ///     If successfully created, a named portal object lasts till the end of the current transaction, unless explicitly destroyed.
    ///     An unnamed portal is destroyed at the end of the transaction, or as soon as the next Bind statement specifying the unnamed portal as destination is issued
    ///
    /// https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-EXT-QUERY
    ///
    pub fn complete_execution(&mut self) {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Execute complete");

        if let Some(execute) = self.get_execute() {
            // Get labels from current session metadata
            let (statement_type, protocol, mapped, multi_statement) = if let Some(session) = self.get_session_metrics() {
                let metadata = &session.metadata;
                (
                    metadata.statement_type.map(|t| t.as_label()).unwrap_or("unknown"),
                    metadata.protocol.map(|p| p.as_label()).unwrap_or("unknown"),
                    if metadata.encrypted { "true" } else { "false" },
                    if metadata.multi_statement { "true" } else { "false" },
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
            ).record(execute.duration());

            if execute.name.is_unnamed() {
                self.close_portal(&execute.name);
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
    }

    pub fn add_portal(&mut self, name: Name, portal: Portal) {
        debug!(target: CONTEXT, client_id = self.client_id, name = ?name, portal = ?portal);
        let _ = self.portals.write().map(|mut portals| {
            portals
                .entry(name)
                .or_insert_with(Queue::new)
                .add(Arc::new(portal));
        });
    }

    pub fn get_statement(&self, name: &Name) -> Option<Arc<Statement>> {
        debug!(target: CONTEXT, client_id = self.client_id, statement = ?name);
        let statements = self.statements.read().ok()?;
        statements.get(name).cloned()
    }

    ///
    /// Close the portal identified by `name`
    /// Portal is removed from  queue
    ///
    pub fn close_portal(&mut self, name: &Name) {
        debug!(target: CONTEXT, client_id = self.client_id, msg = "Close Portal", name = ?name);
        let _ = self.portals.write().map(|mut portals| {
            portals
                .entry(name.clone())
                .and_modify(|queue| queue.complete());
        });
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
            Portal::Passthrough => None,
        }
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
    ) -> Result<Vec<Option<crate::EqlCiphertext>>, Error> {
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

    /// Record parse phase duration for the current session (first write wins)
    pub fn record_parse_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.record_parse(duration);
            }
        }
    }

    /// Add encrypt phase duration for the current session (accumulate)
    pub fn add_encrypt_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.add_encrypt(duration);
            }
        }
    }

    /// Record server write phase duration
    pub fn record_server_write_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.record_server_write(duration);
            }
        }
    }

    /// Add server write phase duration (accumulate)
    pub fn add_server_write_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.add_server_write(duration);
            }
        }
    }

    /// Record server wait phase duration (time to first response byte)
    pub fn record_server_wait_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.record_server_wait(duration);
            }
        }
    }

    /// Record server response phase duration
    pub fn record_server_response_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.record_server_response(duration);
            }
        }
    }

    /// Add server response phase duration (accumulate)
    pub fn add_server_response_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.add_server_response(duration);
            }
        }
    }

    /// Record client write phase duration
    pub fn record_client_write_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.record_client_write(duration);
            }
        }
    }

    /// Add client write phase duration (accumulate)
    pub fn add_client_write_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.add_client_write(duration);
            }
        }
    }

    /// Add decrypt phase duration (accumulate)
    pub fn add_decrypt_duration(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                session.phase_timing.add_decrypt(duration);
            }
        }
    }

    /// Update statement metadata for the current session
    pub fn update_statement_metadata<F>(&mut self, f: F)
    where
        F: FnOnce(&mut StatementMetadata),
    {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                f(&mut session.metadata);
            }
        }
    }

    /// Record server wait for first response; otherwise accumulate response time
    pub fn record_server_wait_or_add_response(&mut self, duration: Duration) {
        if let Ok(mut queue) = self.session_metrics.write() {
            if let Some(session) = queue.current_mut() {
                if session.phase_timing.server_wait_duration.is_none() {
                    session.phase_timing.record_server_wait(duration);
                } else {
                    session.phase_timing.add_server_response(duration);
                }
            }
        }
    }
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Queue {
            queue: VecDeque::new(),
        }
    }

    pub fn complete(&mut self) {
        let _ = self.queue.pop_front();
    }

    pub fn next(&self) -> Option<&T> {
        self.queue.front()
    }

    pub fn add(&mut self, item: T) {
        self.queue.push_back(item);
    }

    /// Get mutable reference to the current (first) item in the queue
    pub fn current_mut(&mut self) -> Option<&mut T> {
        self.queue.front_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, Describe, KeysetIdentifier, Portal, Statement};
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
    use cipherstash_client::IdentifiedBy;
    use eql_mapper::Schema;
    use sqltk::parser::{dialect::PostgreSqlDialect, parser::Parser};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use uuid::Uuid;

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
        Portal::encrypted_with_format_codes(statement.clone(), vec![])
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

        let statement_name = Name::from("statement");
        let portal_name = Name::from("portal");

        // Add statement to context
        context.add_statement(statement_name.clone(), statement());

        // Get statement from context
        let statement = context.get_statement(&statement_name).unwrap();

        // Add portal pointing to statement to context
        context.add_portal(portal_name.clone(), portal(&statement));

        // Add statement name to execute context
        context.set_execute(portal_name.clone());

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
        context.set_execute(portal_name.clone());
        context.set_execute(portal_name.clone());

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
        context.set_execute(portal_name_1.clone());
        context.set_execute(portal_name_2.clone());
        context.set_execute(portal_name_3.clone());

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
