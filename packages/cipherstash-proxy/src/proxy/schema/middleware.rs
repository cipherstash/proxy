//! Transaction-aware coordination between PostgreSQL protocol events and schema mapping.
//!
//! # Principles of operation
//!
//! PostgreSQL is the authority. Proxy never publishes a schema inferred from client SQL to
//! other connections. Instead, [`CommittedSchemaStore`] exposes immutable, monotonically
//! versioned snapshots loaded from PostgreSQL. Each snapshot contains both structural schema
//! and encryption metadata, so a mapper can never observe a table definition from one catalog
//! generation and encryption rules from another.
//!
//! A connection adopts a committed snapshot while idle and pins it when work begins. Confirmed
//! DDL is applied to a connection-local overlay, producing the connection's effective schema:
//! the pinned committed snapshot plus successful changes in the current transaction. Savepoints
//! checkpoint both the structural overlay and encryption metadata. Full rollback discards the
//! overlay; rollback to a savepoint restores its checkpoint; release keeps the changes in the
//! enclosing transaction.
//!
//! Protocol events, rather than parsing alone, drive state changes:
//!
//! 1. `Parse` records DDL intent under the prepared-statement name.
//! 2. `Bind` associates that intent with a portal.
//! 3. `Execute` queues the intent and marks DDL as in flight.
//! 4. Backend success activates the change; backend failure discards it.
//! 5. `Sync` and simple-query boundaries segment the execution queue because PostgreSQL skips
//!    the rest of an extended-protocol batch after an error.
//!
//! Schema-dependent work waits behind in-flight DDL, which makes pipelining correct without
//! guessing whether PostgreSQL will accept the change. At outermost commit, the connection asks
//! the schema manager to reload PostgreSQL before successful idle readiness reaches the client.
//! Publication requests are shared, coalesced, and generation ordered. If publication fails,
//! the committed database transaction cannot be undone; Proxy therefore retains the pending
//! publication, withholds successful readiness, and closes that client connection.
//!
//! # Trade-offs and limitations
//!
//! * The overlay intentionally models a small, deterministic DDL subset. Conditional DDL,
//!   cascading changes, `CREATE TABLE AS`/`LIKE`/`CLONE`/`INHERITS`, views, and unsupported
//!   `ALTER TABLE` operations are treated as unmodelled. After such DDL succeeds, later
//!   schema-dependent statements fail closed until rollback or authoritative publication.
//! * A simple-query message has one PostgreSQL response boundary, so Proxy cannot safely remap a
//!   later statement after observing an earlier statement's result. Batches that may change
//!   encryption metadata and then perform schema-dependent work are rejected. Explicitly native
//!   table DDL remains compatible because it cannot create an encryption obligation.
//! * Temporary and other connection-local catalog objects are invisible to the separate
//!   publication connection. Native temporary-table batches may pass through, but encrypted
//!   temporary objects cannot be represented or authoritatively published and are unsupported.
//! * SQL executed indirectly by procedures, extensions, or dynamic SQL cannot be inferred from
//!   the wire statement. Periodic authoritative reloads eventually discover committed global
//!   changes; a transaction still keeps its pinned view for consistency.
//! * Prepared-statement and portal intents are retained for the connection lifetime. Reusing a
//!   protocol name replaces its entry, but closing many unique names can retain bounded metadata
//!   until the client disconnects.
//!
//! For supported schema changes, these conservative refusals trade some PostgreSQL surface-area
//! compatibility for the core invariant: Proxy must never forward plaintext because it
//! speculated about uncommitted or incompletely modelled schema state.

use super::eql_domains;
use super::manager::CommittedSchemaStore;
use crate::postgresql::Name;
use crate::proxy::encrypt_config::from_domain::column_config_from_domain;
use crate::proxy::EncryptConfig;
use cipherstash_client::eql::Identifier;
use eql_mapper::{ColumnKind, Schema, SchemaWithEdits, TableResolver};
use sqltk::parser::ast::{
    AlterTableOperation, ColumnDef, DropBehavior, Ident, ObjectName, ObjectNamePart, ObjectType,
    Statement,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, RwLock,
};
use tokio::sync::Notify;

#[derive(Clone, Debug)]
/// The schema-relevant meaning recorded for a parsed statement.
struct Intent {
    statement: Statement,
    ddl: bool,
    modelled: bool,
}

#[derive(Clone, Debug)]
/// One execution or synchronization boundary awaiting a backend outcome.
enum PendingExecution {
    /// An execution, optionally carrying schema intent.
    Execute(Option<Box<Intent>>),
    /// The boundary terminated by the next `ReadyForQuery` message.
    ReadyBoundary,
}

#[derive(Clone, Debug)]
/// A transaction savepoint and its corresponding schema checkpoint.
struct Savepoint {
    name: Ident,
    schema: SchemaWithEdits,
    encrypt_config: EncryptConfig,
    unmodelled: bool,
}

/// Connection-local owner of the effective database schema.
///
/// Protocol adapters report DDL execution outcomes through this interface;
/// parsing alone never changes the resolver visible to later statements.
#[derive(Clone, Debug)]
pub struct SchemaMiddleware {
    store: CommittedSchemaStore,
    base: Arc<RwLock<Arc<Schema>>>,
    base_encrypt_config: Arc<RwLock<Arc<EncryptConfig>>>,
    encrypt_config: Arc<RwLock<Arc<EncryptConfig>>>,
    resolver: Arc<RwLock<Arc<TableResolver>>>,
    prepared: Arc<RwLock<HashMap<Name, Intent>>>,
    portals: Arc<RwLock<HashMap<Name, Intent>>>,
    executions: Arc<RwLock<VecDeque<PendingExecution>>>,
    in_flight_ddl: Arc<AtomicUsize>,
    execution_finished: Arc<Notify>,
    dirty: Arc<AtomicBool>,
    unmodelled: Arc<AtomicBool>,
    transaction_active: Arc<AtomicBool>,
    savepoints: Arc<RwLock<Vec<Savepoint>>>,
}

impl SchemaMiddleware {
    #[cfg(test)]
    /// Constructs middleware around a schema-only snapshot for unit tests.
    pub fn new(schema: Arc<Schema>) -> Self {
        Self::from_store(CommittedSchemaStore::for_testing(
            (*schema).clone(),
            EncryptConfig::new(),
        ))
    }

    /// Constructs connection-local middleware backed by the shared committed store.
    pub fn from_store(store: CommittedSchemaStore) -> Self {
        let snapshot = store.load();
        let schema = snapshot.schema();
        Self {
            store,
            base: Arc::new(RwLock::new(schema.clone())),
            base_encrypt_config: Arc::new(RwLock::new(snapshot.encrypt_config())),
            encrypt_config: Arc::new(RwLock::new(snapshot.encrypt_config())),
            resolver: Arc::new(RwLock::new(Arc::new(TableResolver::new_editable(schema)))),
            prepared: Arc::new(RwLock::new(HashMap::new())),
            portals: Arc::new(RwLock::new(HashMap::new())),
            executions: Arc::new(RwLock::new(VecDeque::new())),
            in_flight_ddl: Arc::new(AtomicUsize::new(0)),
            execution_finished: Arc::new(Notify::new()),
            dirty: Arc::new(AtomicBool::new(false)),
            unmodelled: Arc::new(AtomicBool::new(false)),
            transaction_active: Arc::new(AtomicBool::new(false)),
            savepoints: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Returns the resolver for the connection's current effective schema.
    pub fn resolver(&self) -> Arc<TableResolver> {
        self.resolver.read().unwrap().clone()
    }

    /// Returns encryption metadata aligned with the current effective schema.
    pub fn encrypt_config(&self) -> Arc<EncryptConfig> {
        self.encrypt_config.read().unwrap().clone()
    }

    /// Replaces connection-local state with the latest committed snapshot.
    pub fn adopt_latest(&self) {
        let snapshot = self.store.load();
        let schema = snapshot.schema();
        *self.base.write().unwrap() = schema.clone();
        let encrypt_config = snapshot.encrypt_config();
        *self.base_encrypt_config.write().unwrap() = encrypt_config.clone();
        *self.encrypt_config.write().unwrap() = encrypt_config;
        *self.resolver.write().unwrap() = Arc::new(TableResolver::new_editable(schema));
        self.savepoints.write().unwrap().clear();
        self.unmodelled.store(false, Ordering::Release);
    }

    /// Returns whether authoritative catalog publication is still required.
    pub fn needs_publication(&self) -> bool {
        self.dirty.load(Ordering::Acquire) || self.store.publication_pending()
    }

    /// Returns whether this connection has confirmed, unpublished DDL changes.
    pub fn has_local_changes(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    /// Records a shared publication request for the connection's committed DDL.
    pub fn mark_publication_pending(&self) {
        self.store.mark_publication_pending();
    }

    /// Returns whether an idle connection must publish pending catalog state before use.
    pub fn requires_publication_before_statement(&self) -> bool {
        !self.transaction_active.load(Ordering::Acquire)
            && !self.has_local_changes()
            && self.store.publication_pending()
    }

    /// Clears local dirty state and adopts the newly published snapshot.
    pub fn publication_succeeded(&self) {
        self.dirty.store(false, Ordering::Release);
        self.adopt_latest();
    }

    /// Returns whether confirmed DDL cannot be represented by the local overlay.
    pub fn has_unmodelled_ddl(&self) -> bool {
        self.unmodelled.load(Ordering::Acquire)
    }

    /// Adopts a newer committed snapshot before an idle connection starts work.
    pub fn before_statement(&self) {
        if !self.transaction_active.load(Ordering::Acquire)
            && !self.has_local_changes()
            && !self.store.publication_pending()
            && self.in_flight_ddl.load(Ordering::Acquire) == 0
        {
            self.adopt_latest();
        }
    }

    /// Applies a PostgreSQL readiness boundary and its transaction status.
    pub fn ready_for_query(&self, status: u8) {
        self.discard_skipped_executions();
        self.transaction_active
            .store(status != b'I', Ordering::Release);
        if status == b'I' && !self.needs_publication() {
            self.adopt_latest();
        }
    }

    /// Records schema intent for a named prepared statement.
    pub fn prepare(&self, name: Name, statement: Statement) {
        self.prepared.write().unwrap().insert(
            name,
            Intent {
                ddl: is_schema_ddl(&statement),
                modelled: is_modelled_ddl(&statement),
                statement,
            },
        );
    }

    /// Associates a portal with the schema intent of its prepared statement.
    pub fn bind(&self, portal: Name, prepared_statement: &Name) {
        let intent = self
            .prepared
            .read()
            .unwrap()
            .get(prepared_statement)
            .cloned();
        let mut portals = self.portals.write().unwrap();
        match intent {
            Some(intent) => {
                portals.insert(portal, intent);
            }
            None => {
                portals.remove(&portal);
            }
        }
    }

    /// Records a portal execution awaiting a backend success or failure response.
    pub fn execute(&self, portal: &Name) {
        let intent = self.portals.read().unwrap().get(portal).cloned();
        self.transaction_active.store(true, Ordering::Release);
        if intent.as_ref().is_some_and(|intent| intent.ddl) {
            self.in_flight_ddl.fetch_add(1, Ordering::AcqRel);
        }
        self.executions
            .write()
            .unwrap()
            .push_back(PendingExecution::Execute(intent.map(Box::new)));
    }

    /// Records every statement and readiness boundary in a simple-query message.
    pub fn simple_query(&self, statements: &[Statement]) {
        self.transaction_active.store(true, Ordering::Release);
        let mut executions = self.executions.write().unwrap();
        for statement in statements {
            let intent = Intent {
                ddl: is_schema_ddl(statement),
                modelled: is_modelled_ddl(statement),
                statement: statement.clone(),
            };
            if intent.ddl {
                self.in_flight_ddl.fetch_add(1, Ordering::AcqRel);
            }
            executions.push_back(PendingExecution::Execute(Some(Box::new(intent))));
        }
        executions.push_back(PendingExecution::ReadyBoundary);
    }

    /// Returns whether mapping a later statement in this simple-query batch
    /// could observe encryption metadata changed by an earlier DDL statement.
    pub fn simple_query_requires_fail_closed(&self, statements: &[Statement]) -> bool {
        let mut encryption_changing_ddl_seen = false;

        for statement in statements {
            if encryption_changing_ddl_seen && eql_mapper::requires_type_check(statement) {
                return true;
            }
            encryption_changing_ddl_seen |= self.ddl_may_change_encryption(statement);
        }

        false
    }

    /// Conservatively classifies DDL that can change encryption metadata.
    fn ddl_may_change_encryption(&self, statement: &Statement) -> bool {
        match statement {
            Statement::CreateTable(create) => {
                create.query.is_some()
                    || create.like.is_some()
                    || create.clone.is_some()
                    || create.inherits.is_some()
                    || create
                        .columns
                        .iter()
                        .any(|column| matches!(column_kind(column), ColumnKind::Eql(_, _)))
            }
            Statement::AlterTable {
                name, operations, ..
            } => {
                self.encrypt_config().contains_table(object_name(name))
                    || operations.iter().any(|operation| match operation {
                        AlterTableOperation::AddColumn { column_def, .. } => {
                            matches!(column_kind(column_def), ColumnKind::Eql(_, _))
                        }
                        AlterTableOperation::DropColumn { .. }
                        | AlterTableOperation::RenameColumn { .. }
                        | AlterTableOperation::RenameTable { .. } => false,
                        _ => true,
                    })
            }
            Statement::Drop {
                object_type: ObjectType::Table,
                names,
                ..
            } => names
                .iter()
                .any(|name| self.encrypt_config().contains_table(object_name(name))),
            Statement::CreateView { .. }
            | Statement::Drop {
                object_type: ObjectType::View,
                ..
            } => true,
            _ => false,
        }
    }

    /// Marks the `Sync` boundary whose `ReadyForQuery` terminates an extended
    /// protocol batch. PostgreSQL skips the remaining executions in that batch
    /// after an error, but may already have a later batch queued behind it.
    pub fn protocol_boundary(&self) {
        self.executions
            .write()
            .unwrap()
            .push_back(PendingExecution::ReadyBoundary);
    }

    /// Waits until all earlier schema-changing executions have resolved.
    pub async fn wait_for_ddl(&self) {
        loop {
            let notified = self.execution_finished.notified();
            if self.in_flight_ddl.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }

    #[cfg(test)]
    /// Records a direct statement execution for state-machine tests.
    pub fn execution_started(&self, statement: Statement) {
        let ddl = is_schema_ddl(&statement);
        if ddl {
            self.in_flight_ddl.fetch_add(1, Ordering::AcqRel);
        }
        self.executions
            .write()
            .unwrap()
            .push_back(PendingExecution::Execute(Some(Box::new(Intent {
                modelled: is_modelled_ddl(&statement),
                statement,
                ddl,
            }))));
    }

    /// Applies the next queued execution after PostgreSQL confirms success.
    pub fn execution_succeeded(&self) {
        if let Some(Some(intent)) = self.pop_execution() {
            let statement = intent.statement;
            match statement {
                Statement::Savepoint { name } => {
                    let resolver = self.resolver();
                    let overlay = resolver.as_schema_with_edits().unwrap();
                    let checkpoint = overlay.read().unwrap().clone();
                    self.savepoints.write().unwrap().push(Savepoint {
                        name,
                        schema: checkpoint,
                        encrypt_config: (*self.encrypt_config()).clone(),
                        unmodelled: self.has_unmodelled_ddl(),
                    });
                }
                Statement::ReleaseSavepoint { name } => {
                    let mut savepoints = self.savepoints.write().unwrap();
                    if let Some(index) = savepoints
                        .iter()
                        .rposition(|savepoint| savepoint.name == name)
                    {
                        savepoints.truncate(index);
                    }
                }
                Statement::Rollback {
                    savepoint: Some(name),
                    ..
                } => {
                    let mut savepoints = self.savepoints.write().unwrap();
                    if let Some(index) = savepoints
                        .iter()
                        .rposition(|savepoint| savepoint.name == name)
                    {
                        let checkpoint = savepoints[index].schema.clone();
                        let encrypt_config = savepoints[index].encrypt_config.clone();
                        let unmodelled = savepoints[index].unmodelled;
                        savepoints.truncate(index + 1);
                        let resolver = self.resolver();
                        let overlay = resolver.as_schema_with_edits().unwrap();
                        *overlay.write().unwrap() = checkpoint;
                        *self.encrypt_config.write().unwrap() = Arc::new(encrypt_config);
                        self.unmodelled.store(unmodelled, Ordering::Release);
                        self.dirty.store(
                            unmodelled || resolver.has_schema_changed(),
                            Ordering::Release,
                        );
                    }
                }
                Statement::Rollback {
                    savepoint: None, ..
                } => {
                    *self.resolver.write().unwrap() = Arc::new(TableResolver::new_editable(
                        self.base.read().unwrap().clone(),
                    ));
                    *self.encrypt_config.write().unwrap() =
                        self.base_encrypt_config.read().unwrap().clone();
                    self.savepoints.write().unwrap().clear();
                    self.dirty.store(false, Ordering::Release);
                    self.unmodelled.store(false, Ordering::Release);
                }
                statement => {
                    if intent.ddl && !intent.modelled {
                        self.unmodelled.store(true, Ordering::Release);
                    } else {
                        self.apply_ddl(&statement);
                    }
                }
            }
            if intent.ddl {
                self.dirty.store(
                    self.has_unmodelled_ddl() || self.resolver().has_schema_changed(),
                    Ordering::Release,
                );
                self.in_flight_ddl.fetch_sub(1, Ordering::AcqRel);
                self.execution_finished.notify_waiters();
            }
        }
    }

    /// Applies modelled DDL to schema and encryption overlays as one operation.
    fn apply_ddl(&self, statement: &Statement) {
        eql_mapper::collect_ddl_with_column_kind(self.resolver(), statement, &|column| {
            column_kind(column)
        });

        let mut config = (*self.encrypt_config()).clone();
        apply_encrypt_config(&mut config, statement);
        *self.encrypt_config.write().unwrap() = Arc::new(config);
    }

    /// Discards the next queued execution after PostgreSQL reports failure.
    pub fn execution_failed(&self) {
        if let Some(Some(intent)) = self.pop_execution() {
            if intent.ddl {
                self.in_flight_ddl.fetch_sub(1, Ordering::AcqRel);
                self.execution_finished.notify_waiters();
            }
        }
    }

    /// Removes the next execution without crossing a readiness boundary.
    fn pop_execution(&self) -> Option<Option<Intent>> {
        let mut executions = self.executions.write().unwrap();
        if matches!(executions.front(), Some(PendingExecution::Execute(_))) {
            match executions.pop_front().unwrap() {
                PendingExecution::Execute(intent) => Some(intent.map(|intent| *intent)),
                PendingExecution::ReadyBoundary => unreachable!(),
            }
        } else {
            None
        }
    }

    /// Discards executions PostgreSQL skipped after an error up to readiness.
    fn discard_skipped_executions(&self) {
        let mut discarded_ddl = 0;
        let mut executions = self.executions.write().unwrap();
        while let Some(execution) = executions.pop_front() {
            match execution {
                PendingExecution::Execute(Some(intent)) if intent.ddl => discarded_ddl += 1,
                PendingExecution::Execute(_) => {}
                PendingExecution::ReadyBoundary => break,
            }
        }
        drop(executions);

        if discarded_ddl > 0 {
            self.in_flight_ddl
                .fetch_sub(discarded_ddl, Ordering::AcqRel);
            self.execution_finished.notify_waiters();
        }
    }
}

/// Returns the unqualified PostgreSQL domain name declared for a column.
fn column_domain(column: &ColumnDef) -> String {
    column
        .data_type
        .to_string()
        .split('.')
        .next_back()
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned()
}

/// Classifies a declared column as native or as an EQL domain.
fn column_kind(column: &ColumnDef) -> ColumnKind {
    eql_domains::resolve(&column_domain(column))
        .map(|(identity, traits)| ColumnKind::Eql(traits, identity))
        .unwrap_or(ColumnKind::Native)
}

/// Returns the final identifier component of a PostgreSQL object name.
fn object_name(name: &ObjectName) -> &str {
    match name.0.last() {
        Some(ObjectNamePart::Identifier(name)) => &name.value,
        _ => "",
    }
}

/// Adds inferred encryption metadata for one newly declared column.
fn add_column_config(config: &mut EncryptConfig, table: &str, column: &ColumnDef) {
    let domain = column_domain(column);
    if let Some(column_config) = column_config_from_domain(table, &column.name.value, &domain) {
        config.insert(
            Identifier::new(table.to_owned(), column.name.value.clone()),
            column_config,
        );
    }
}

/// Applies modelled DDL to a mutable encryption-metadata overlay.
fn apply_encrypt_config(config: &mut EncryptConfig, statement: &Statement) {
    match statement {
        Statement::CreateTable(create) => {
            let table = object_name(&create.name);
            config.remove_table(table);
            for column in &create.columns {
                add_column_config(config, table, column);
            }
        }
        Statement::AlterTable {
            name, operations, ..
        } => {
            let table = object_name(name);
            for operation in operations {
                match operation {
                    AlterTableOperation::AddColumn { column_def, .. } => {
                        add_column_config(config, table, column_def);
                    }
                    AlterTableOperation::DropColumn { column_name, .. } => {
                        config.remove_column(table, &column_name.value);
                    }
                    AlterTableOperation::RenameColumn {
                        old_column_name,
                        new_column_name,
                    } => {
                        config.rename_column(table, &old_column_name.value, &new_column_name.value);
                    }
                    AlterTableOperation::RenameTable { table_name } => {
                        config.rename_table(table, object_name(table_name));
                    }
                    _ => {}
                }
            }
        }
        Statement::Drop {
            object_type: ObjectType::Table | ObjectType::View,
            names,
            ..
        } => {
            for name in names {
                config.remove_table(object_name(name));
            }
        }
        _ => {}
    }
}

/// Returns whether a statement changes schema state tracked by the middleware.
fn is_schema_ddl(statement: &Statement) -> bool {
    matches!(
        statement,
        Statement::CreateTable(_)
            | Statement::CreateView { .. }
            | Statement::AlterTable { .. }
            | Statement::Drop {
                object_type: ObjectType::Table | ObjectType::View,
                ..
            }
    )
}

/// Returns whether a DDL statement can be represented exactly by the overlay.
fn is_modelled_ddl(statement: &Statement) -> bool {
    match statement {
        Statement::CreateTable(create) => {
            !create.or_replace
                && !create.if_not_exists
                && !create.temporary
                && create.query.is_none()
                && create.like.is_none()
                && create.clone.is_none()
                && create.inherits.is_none()
                && create.on_commit.is_none()
        }
        Statement::AlterTable { operations, .. } => {
            operations.iter().all(|operation| match operation {
                AlterTableOperation::AddColumn { if_not_exists, .. } => !if_not_exists,
                AlterTableOperation::RenameColumn { .. }
                | AlterTableOperation::RenameTable { .. } => true,
                AlterTableOperation::DropColumn { drop_behavior, .. } => {
                    *drop_behavior != Some(DropBehavior::Cascade)
                }
                _ => false,
            })
        }
        Statement::Drop {
            object_type: ObjectType::Table | ObjectType::View,
            cascade,
            ..
        } => !cascade,
        Statement::CreateView { .. } => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eql_mapper::Table;
    use sqltk::parser::ast::{Ident, ObjectName, ObjectNamePart};
    use sqltk::parser::{dialect::PostgreSqlDialect, parser::Parser};

    fn parse(sql: &str) -> Statement {
        Parser::new(&PostgreSqlDialect {})
            .try_with_sql(sql)
            .unwrap()
            .parse_statement()
            .unwrap()
    }

    fn table(name: &str) -> ObjectName {
        ObjectName(vec![ObjectNamePart::Identifier(Ident::new(name))])
    }

    #[test]
    fn ddl_changes_effective_schema_only_after_successful_execution() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));
        let ddl = parse("create table reports (id bigint)");

        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_err());

        middleware.execution_started(ddl);

        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_err());

        middleware.execution_succeeded();

        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_ok());
    }

    #[test]
    fn full_rollback_discards_successful_transaction_ddl() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));

        middleware.execution_started(parse("create table reports (id bigint)"));
        middleware.execution_succeeded();
        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_ok());

        middleware.execution_started(parse("rollback"));
        middleware.execution_succeeded();

        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_err());
    }

    #[test]
    fn explicit_commit_keeps_changes_dirty_until_authoritative_publication() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));

        middleware.simple_query(&[parse("begin")]);
        middleware.execution_succeeded();
        middleware.execution_started(parse("create table reports (id bigint)"));
        middleware.execution_succeeded();
        middleware.ready_for_query(b'T');
        middleware.simple_query(&[parse("commit")]);
        middleware.execution_succeeded();
        middleware.ready_for_query(b'I');

        assert!(middleware.needs_publication());
        middleware.publication_succeeded();
        assert!(!middleware.needs_publication());
    }

    #[test]
    fn rollback_to_savepoint_restores_the_overlay_checkpoint() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));

        middleware.execution_started(parse("create table accounts (id bigint)"));
        middleware.execution_succeeded();
        middleware.execution_started(parse("savepoint before_reports"));
        middleware.execution_succeeded();
        middleware.execution_started(parse("create table reports (id bigint)"));
        middleware.execution_succeeded();

        middleware.execution_started(parse("rollback to savepoint before_reports"));
        middleware.execution_succeeded();

        assert!(middleware
            .resolver()
            .resolve_table(&table("accounts"))
            .is_ok());
        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_err());
    }

    #[test]
    fn release_savepoint_preserves_changes_in_the_enclosing_transaction() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));

        middleware.execution_started(parse("create table accounts (id bigint)"));
        middleware.execution_succeeded();
        middleware.execution_started(parse("savepoint before_reports"));
        middleware.execution_succeeded();
        middleware.execution_started(parse("create table reports (id bigint)"));
        middleware.execution_succeeded();
        middleware.execution_started(parse("release savepoint before_reports"));
        middleware.execution_succeeded();

        assert!(middleware
            .resolver()
            .resolve_table(&table("accounts"))
            .is_ok());
        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_ok());
        assert!(middleware.needs_publication());
    }

    #[test]
    fn rollback_to_savepoint_preserves_an_earlier_unmodelled_change() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));

        middleware.execution_started(parse("alter table secrets alter column value type text"));
        middleware.execution_succeeded();
        middleware.execution_started(parse("savepoint after_unmodelled"));
        middleware.execution_succeeded();
        middleware.execution_started(parse("create table reports (id bigint)"));
        middleware.execution_succeeded();
        middleware.execution_started(parse("rollback to savepoint after_unmodelled"));
        middleware.execution_succeeded();

        assert!(middleware.has_unmodelled_ddl());
        assert!(middleware.needs_publication());
    }

    #[test]
    fn idle_connection_adopts_a_newly_published_snapshot() {
        let store = CommittedSchemaStore::for_testing(Schema::new("public"), EncryptConfig::new());
        let middleware = SchemaMiddleware::from_store(store.clone());
        let mut published = Schema::new("public");
        published.add_table(Table::new(Ident::new("reports")));

        store.publish_for_testing(published, EncryptConfig::new());
        middleware.adopt_latest();

        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_ok());
    }

    #[test]
    fn active_transaction_keeps_its_pinned_snapshot_until_idle() {
        let store = CommittedSchemaStore::for_testing(Schema::new("public"), EncryptConfig::new());
        let middleware = SchemaMiddleware::from_store(store.clone());
        let begin = parse("begin");
        middleware.simple_query(&[begin]);
        middleware.execution_succeeded();

        let mut published = Schema::new("public");
        published.add_table(Table::new(Ident::new("reports")));
        store.publish_for_testing(published, EncryptConfig::new());
        middleware.before_statement();

        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_err());
        middleware.ready_for_query(b'I');
        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_ok());
    }

    #[test]
    fn ordinary_extended_execution_pins_the_snapshot_until_readiness() {
        let store = CommittedSchemaStore::for_testing(Schema::new("public"), EncryptConfig::new());
        let middleware = SchemaMiddleware::from_store(store.clone());
        let statement = Name::from("statement");
        let portal = Name::from("portal");
        middleware.prepare(statement.clone(), parse("select 1"));
        middleware.bind(portal.clone(), &statement);
        middleware.execute(&portal);

        let mut published = Schema::new("public");
        published.add_table(Table::new(Ident::new("reports")));
        store.publish_for_testing(published, EncryptConfig::new());
        middleware.before_statement();

        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_err());
        middleware.execution_succeeded();
        middleware.protocol_boundary();
        middleware.ready_for_query(b'I');
        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_ok());
    }

    #[test]
    fn publication_failure_is_visible_to_other_connections() {
        let store = CommittedSchemaStore::for_testing(Schema::new("public"), EncryptConfig::new());
        let publisher = SchemaMiddleware::from_store(store.clone());
        let idle_connection = SchemaMiddleware::from_store(store);

        publisher.execution_started(parse("create table reports (id bigint)"));
        publisher.execution_succeeded();
        publisher.mark_publication_pending();

        assert!(idle_connection.requires_publication_before_statement());
    }

    #[test]
    fn successful_ddl_activates_schema_and_encryption_metadata_together() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));

        middleware.execution_started(parse(
            "create table secrets (id bigint, value eql_v3_text_search)",
        ));
        middleware.execution_succeeded();

        let column = middleware
            .resolver()
            .resolve_table_column(&table("secrets"), &Ident::new("value"))
            .unwrap();
        assert!(matches!(column.kind, ColumnKind::Eql(_, _)));
        assert!(middleware
            .encrypt_config()
            .get_column_config(&Identifier::new("secrets".to_owned(), "value".to_owned()))
            .is_some());
    }

    #[test]
    fn successful_unmodelled_ddl_refuses_later_schema_use_until_rollback() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));

        middleware.execution_started(parse("alter table secrets alter column value type text"));
        middleware.execution_succeeded();

        assert!(middleware.has_unmodelled_ddl());
        assert!(middleware.needs_publication());

        middleware.execution_started(parse("rollback"));
        middleware.execution_succeeded();
        assert!(!middleware.has_unmodelled_ddl());
    }

    #[test]
    fn create_table_as_is_unmodelled() {
        assert!(!is_modelled_ddl(&parse(
            "create table archived_reports as select 1 as id",
        )));
    }

    #[test]
    fn cascading_column_drop_is_unmodelled() {
        assert!(!is_modelled_ddl(&parse(
            "alter table reports drop column account_id cascade",
        )));
    }

    #[test]
    fn conditional_create_and_add_are_unmodelled() {
        assert!(!is_modelled_ddl(&parse(
            "create table if not exists reports (id bigint)",
        )));
        assert!(!is_modelled_ddl(&parse(
            "alter table reports add column if not exists account_id bigint",
        )));
    }

    #[test]
    fn native_temporary_table_batch_does_not_fail_closed() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));
        let statements = vec![
            parse("create temporary table names (name text)"),
            parse("insert into names (name) values ('Ada')"),
        ];

        assert!(!middleware.simple_query_requires_fail_closed(&statements));
    }

    #[test]
    fn encrypted_table_batch_fails_closed_before_dependent_insert() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));
        let statements = vec![
            parse("create table secrets (value eql_v3_text_search)"),
            parse("insert into secrets (value) values ('classified')"),
        ];

        assert!(middleware.simple_query_requires_fail_closed(&statements));
    }

    #[tokio::test]
    async fn dependent_mapping_waits_for_ddl_execution_outcome() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));
        middleware.execution_started(parse("create table reports (id bigint)"));

        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            middleware.wait_for_ddl(),
        )
        .await
        .is_err());

        middleware.execution_failed();

        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            middleware.wait_for_ddl(),
        )
        .await
        .unwrap();
        assert!(middleware
            .resolver()
            .resolve_table(&table("reports"))
            .is_err());
    }

    #[tokio::test]
    async fn readiness_discards_pipelined_executions_skipped_after_an_error() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));
        middleware.execution_started(parse("create table first_table (id bigint)"));
        middleware.execution_started(parse("create table skipped_table (id bigint)"));
        middleware.protocol_boundary();

        middleware.execution_failed();
        middleware.ready_for_query(b'I');

        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            middleware.wait_for_ddl(),
        )
        .await
        .unwrap();
        assert!(middleware
            .resolver()
            .resolve_table(&table("skipped_table"))
            .is_err());
    }

    #[tokio::test]
    async fn readiness_does_not_discard_a_later_pipelined_batch() {
        let middleware = SchemaMiddleware::new(Arc::new(Schema::new("public")));
        middleware.execution_started(parse("create table failed_table (id bigint)"));
        middleware.execution_started(parse("create table skipped_table (id bigint)"));
        middleware.protocol_boundary();
        middleware.execution_started(parse("create table later_table (id bigint)"));
        middleware.protocol_boundary();

        middleware.execution_failed();
        middleware.ready_for_query(b'I');

        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            middleware.wait_for_ddl(),
        )
        .await
        .is_err());
        middleware.execution_succeeded();
        middleware.ready_for_query(b'I');
        middleware.wait_for_ddl().await;
        assert!(middleware
            .resolver()
            .resolve_table(&table("later_table"))
            .is_ok());
    }
}
