pub mod column;

use super::{
    format_code::FormatCode,
    messages::{
        describe::{Describe, Target},
        Name,
    },
    Column,
};
use crate::{
    error::{EncryptError, Error},
    log::CONTEXT,
    prometheus::{STATEMENTS_EXECUTION_DURATION_SECONDS, STATEMENTS_SESSION_DURATION_SECONDS},
};
use cipherstash_client::IdentifiedBy;
use eql_mapper::{Schema, TableResolver};
use metrics::histogram;
use sqltk::parser::ast::{Expr, Ident, ObjectName, ObjectNamePart, Set, Value, ValueWithSpan};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, RwLock},
    time::{Duration, Instant},
};
use tracing::{debug, warn};
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

#[derive(Clone, Debug)]
pub struct Context {
    pub client_id: i32,
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
}

impl SessionMetricsContext {
    fn new() -> SessionMetricsContext {
        SessionMetricsContext {
            start: Instant::now(),
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

///
/// Type Analysed parameters and projection
///
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub param_columns: Vec<Option<Column>>,
    pub projection_columns: Vec<Option<Column>>,
    pub literal_columns: Vec<Option<Column>>,
    pub postgres_param_types: Vec<i32>,
}

#[derive(Clone, Debug)]
pub enum Portal {
    Encrypted {
        format_codes: Vec<FormatCode>,
        statement: Arc<Statement>,
    },
    Passthrough,
}

impl Context {
    pub fn new(client_id: i32, schema: Arc<Schema>) -> Context {
        Context {
            statements: Arc::new(RwLock::new(HashMap::new())),
            portals: Arc::new(RwLock::new(HashMap::new())),
            describe: Arc::new(RwLock::from(Queue::new())),
            execute: Arc::new(RwLock::from(Queue::new())),
            schema_changed: Arc::new(RwLock::from(false)),
            session_metrics: Arc::new(RwLock::from(Queue::new())),
            table_resolver: Arc::new(TableResolver::new_editable(schema)),
            client_id,
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
            histogram!(STATEMENTS_SESSION_DURATION_SECONDS).record(session.duration());
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
            histogram!(STATEMENTS_EXECUTION_DURATION_SECONDS).record(execute.duration());
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
                target: Target::PreparedStatement,
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
                if let Some(Expr::Value(ValueWithSpan { value, .. })) = values.first() {
                    let keyset_name = match value {
                        Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => s.clone(),
                        Value::Number(n, _) => n.to_string(),
                        _ => {
                            let err = EncryptError::KeysetNameCouldNotBeSet;
                            warn!(target: CONTEXT, client_id = self.client_id, msg = err.to_string());
                            return Ok(None);
                        }
                    };

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
                    // We let the database handle any syntax errors to avoid complexifying the fronted flow (more)
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
}

impl Statement {
    pub fn new(
        param_columns: Vec<Option<Column>>,
        projection_columns: Vec<Option<Column>>,
        literal_columns: Vec<Option<Column>>,
        postgres_param_types: Vec<i32>,
    ) -> Statement {
        Statement {
            param_columns,
            projection_columns,
            literal_columns,
            postgres_param_types,
        }
    }

    pub fn unencryped() -> Statement {
        Statement::new(vec![], vec![], vec![], vec![])
    }

    pub fn has_literals(&self) -> bool {
        !self.literal_columns.is_empty()
    }

    pub fn has_params(&self) -> bool {
        !self.param_columns.is_empty()
    }

    pub fn has_projection(&self) -> bool {
        !self.projection_columns.is_empty()
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
}

impl Portal {
    pub fn encrypted_with_format_codes(
        statement: Arc<Statement>,
        format_codes: Vec<FormatCode>,
    ) -> Portal {
        Portal::Encrypted {
            statement,
            format_codes,
        }
    }

    pub fn encrypted(statement: Arc<Statement>) -> Portal {
        let format_codes = vec![];
        Portal::Encrypted {
            statement,
            format_codes,
        }
    }

    pub fn passthrough() -> Portal {
        Portal::Passthrough
    }

    pub fn projection_columns(&self) -> &Vec<Option<Column>> {
        static EMPTY: Vec<Option<Column>> = vec![];
        match self {
            Portal::Encrypted { statement, .. } => &statement.projection_columns,
            _ => &EMPTY,
        }
    }

    // FormatCodes should not be None at this point
    // FormatCodes will be:
    //  - empty, in which case assume Text
    //  - single value, in which case use this for all columns
    //  - multiple values, in which case use the value for each column
    pub fn format_codes(&self, row_len: usize) -> Vec<FormatCode> {
        match self {
            Portal::Encrypted { format_codes, .. } => match format_codes.len() {
                0 => vec![FormatCode::Text; row_len],
                1 => {
                    let format_code = match format_codes.first() {
                        Some(code) => *code,
                        None => FormatCode::Text,
                    };
                    vec![format_code; row_len]
                }
                _ => format_codes.clone(),
            },
            Portal::Passthrough => {
                unreachable!()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, Describe, KeysetIdentifier, Portal, Statement};
    use crate::{
        config::LogConfig,
        log,
        postgresql::messages::{describe::Target, Name},
    };
    use cipherstash_client::IdentifiedBy;
    use eql_mapper::Schema;
    use sqltk::parser::{dialect::PostgreSqlDialect, parser::Parser};
    use std::sync::Arc;
    use uuid::Uuid;

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

        let schema = Arc::new(Schema::new("public"));

        let mut context = Context::new(1, schema);

        let name = Name::from("name");

        context.add_statement(name.clone(), statement());

        let statement = context.get_statement(&name).unwrap();

        let describe = Describe {
            name,
            target: Target::PreparedStatement,
        };
        context.set_describe(describe);

        let s = context.get_statement_from_describe().unwrap();

        assert_eq!(s, statement)
    }

    #[test]
    pub fn execution_flow() {
        log::init(LogConfig::default());

        let schema = Arc::new(Schema::new("public"));

        let mut context = Context::new(1, schema);

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

        let schema = Arc::new(Schema::new("public"));

        let mut context = Context::new(1, schema);

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

        let schema = Arc::new(Schema::new("public"));

        let mut context = Context::new(1, schema);

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

        let schema = Arc::new(Schema::new("public"));
        let mut context = Context::new(1, schema);

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

        let schema = Arc::new(Schema::new("public"));

        let uuid = Uuid::parse_str("7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590").unwrap();

        let identifier = KeysetIdentifier(IdentifiedBy::Uuid(uuid));

        let sql = vec![
            "SET CIPHERSTASH.KEYSET_ID = '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'",
            "SET SESSION CIPHERSTASH.KEYSET_ID = '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'",
            "SET CIPHERSTASH.KEYSET_ID TO '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'",
        ];

        for s in sql {
            let mut context = Context::new(1, schema.clone());
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

        let schema = Arc::new(Schema::new("public"));
        let mut context = Context::new(1, schema);

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

        let schema = Arc::new(Schema::new("public"));

        let sql = vec![
            "SET CIPHERSTASH.KEYSET_NAME = 'test-keyset'",
            "SET SESSION CIPHERSTASH.KEYSET_NAME = 'test-keyset'",
            "SET CIPHERSTASH.KEYSET_NAME TO 'test-keyset'",
        ];

        for s in sql {
            let mut context = Context::new(1, schema.clone());
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

        let schema = Arc::new(Schema::new("public"));
        let mut context = Context::new(1, schema);

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

        let schema = Arc::new(Schema::new("public"));

        // Test keyset name with number
        let mut context = Context::new(1, schema.clone());
        let sql = "SET CIPHERSTASH.KEYSET_NAME = 12345";
        let statement = parse_statement(sql);

        let identifier = KeysetIdentifier(IdentifiedBy::Name("12345".to_string().into()));
        let result = context.maybe_set_keyset_name(&statement);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
        assert_eq!(Some(identifier.clone()), context.keyset_identifier());

        // Test keyset id with numeric UUID (should work if it's a valid UUID)
        let mut context = Context::new(2, schema);
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

        let schema = Arc::new(Schema::new("public"));

        // Test that maybe_set_keyset handles both ID and name
        let mut context = Context::new(1, schema.clone());

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
        let mut context = Context::new(2, schema.clone());
        let keyset_name_sql = "SET CIPHERSTASH.KEYSET_NAME = 'test-keyset'";
        let statement = parse_statement(keyset_name_sql);

        let identifier = KeysetIdentifier(IdentifiedBy::Name("test-keyset".to_string().into()));
        let result = context.maybe_set_keyset(&statement);

        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
        assert_eq!(Some(identifier.clone()), context.keyset_identifier());

        // Test with unknown command
        let mut context = Context::new(3, schema);
        let unknown_sql = "SET CIPHERSTASH.UNKNOWN = 'value'";
        let statement = parse_statement(unknown_sql);
        let result = context.maybe_set_keyset(&statement);

        assert!(result.is_ok());
        let identifier = result.unwrap();
        assert!(identifier.is_none());
    }

    #[test]
    pub fn test_unnamed_vs_named_statements() {
        log::init(LogConfig::default());

        let schema = Arc::new(Schema::new("public"));
        let mut context = Context::new(1, schema);

        // Test unnamed statements (go to queue)
        let unnamed_name = Name::unnamed();
        let statement1 = statement();
        let statement2 = statement();

        context.add_statement(&unnamed_name, statement1);
        context.add_statement(&unnamed_name, statement2);

        // Should get the same statement (front of queue) until consumed
        let _retrieved1 = context.get_statement(&unnamed_name).unwrap();
        let _retrieved2 = context.get_statement(&unnamed_name).unwrap();

        // Should still return the first statement (not consumed yet)
        let retrieved3 = context.get_statement(&unnamed_name);
        assert!(retrieved3.is_some());

        // Now consume the first statement
        context.close_statement(&unnamed_name);

        // Should now return the second statement
        let retrieved4 = context.get_statement(&unnamed_name);
        assert!(retrieved4.is_some());

        // Consume the second statement
        context.close_statement(&unnamed_name);

        // Should now return None (queue exhausted)
        let retrieved5 = context.get_statement(&unnamed_name);
        assert!(retrieved5.is_none());

        // Test named statements (go to prepared_statements)
        let named_name1 = Name::from("stmt1");
        let named_name2 = Name::from("stmt2");
        let named_statement1 = statement();
        let named_statement2 = statement();

        context.add_statement(&named_name1, named_statement1);
        context.add_statement(&named_name2, named_statement2);

        // Should be able to get named statements multiple times
        let named_retrieved1 = context.get_statement(&named_name1).unwrap();
        let named_retrieved1_again = context.get_statement(&named_name1).unwrap();
        let named_retrieved2 = context.get_statement(&named_name2).unwrap();

        // Should be able to get same statement multiple times
        assert!(named_retrieved1.param_columns == named_retrieved1_again.param_columns);

        // Close a named statement
        context.close_prepared_statement(&named_name1);
        let should_be_none = context.get_statement(&named_name1);
        assert!(should_be_none.is_none());

        // Other named statement should still be available
        let named_retrieved2_again = context.get_statement(&named_name2).unwrap();
        assert!(named_retrieved2.param_columns == named_retrieved2_again.param_columns);
    }
}
