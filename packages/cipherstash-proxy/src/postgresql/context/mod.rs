pub mod column;

use super::{
    format_code::FormatCode,
    messages::{
        describe::{Describe, Target},
        Name,
    },
    Column,
};
use crate::log::CONTEXT;
use eql_mapper::{Schema, TableResolver};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
};
use tracing::debug;

type DescribeQueue = Queue<Describe>;
type ExecuteQueue = Queue<Name>;
type PortalQueue = Queue<Arc<Portal>>;

#[derive(Clone, Debug)]
pub struct Context {
    pub client_id: i32,
    statements: Arc<RwLock<HashMap<Name, Arc<Statement>>>>,
    portals: Arc<RwLock<HashMap<Name, PortalQueue>>>,
    describe: Arc<RwLock<DescribeQueue>>,
    execute: Arc<RwLock<ExecuteQueue>>,
    schema_changed: Arc<RwLock<bool>>,
    table_resolver: Arc<TableResolver>,
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
    Encrypted(EncryptedPortal),
    Passthrough,
}
///
/// Portal is a Statement with Bound variables
/// An Execute message will execute the statement with the variables associated during a Bind.
///
#[derive(Clone, Debug)]
pub struct EncryptedPortal {
    pub format_codes: Vec<FormatCode>,
    pub statement: Arc<Statement>,
}

impl Context {
    pub fn new(client_id: i32, schema: Arc<Schema>) -> Context {
        Context {
            statements: Arc::new(RwLock::new(HashMap::new())),
            portals: Arc::new(RwLock::new(HashMap::new())),
            describe: Arc::new(RwLock::from(Queue::new())),
            execute: Arc::new(RwLock::from(Queue::new())),
            schema_changed: Arc::new(RwLock::from(false)),
            table_resolver: Arc::new(TableResolver::new_editable(schema)),
            client_id,
        }
    }

    pub fn set_describe(&mut self, describe: Describe) {
        debug!(target: CONTEXT, client_id = self.client_id, "set_describe: {describe:?}");
        let _ = self.describe.write().map(|mut queue| queue.add(describe));
    }
    ///
    /// Marks the current Describe as complete
    /// Removes the Describe from the Queue
    ///
    pub fn complete_describe(&mut self) {
        debug!(target: CONTEXT, client_id = self.client_id, src = "complete_describe");
        let _ = self.describe.write().map(|mut queue| queue.complete());
    }

    pub fn set_execute(&mut self, name: Name) {
        debug!(target: CONTEXT, client_id = self.client_id, "set_execute: {name:?}");
        let _ = self.execute.write().map(|mut queue| queue.add(name));
    }

    ///
    /// Marks the current Execution as Complete
    /// Removes the Named portal from the Queue
    /// Note that the Portal itself is not removed
    ///
    /// If successfully created, a named portal object lasts till the end of the current transaction, unless explicitly destroyed.
    /// An unnamed portal is destroyed at the end of the transaction, or as soon as the next Bind statement specifying the unnamed portal as destination is issued
    pub fn complete_execution(&mut self) {
        debug!(target: CONTEXT, client_id = self.client_id, src = "complete_execution");

        if let Some(name) = self.get_execute() {
            if name.is_unnamed() {
                debug!(target: CONTEXT, client_id = self.client_id, msg= "Close unnamed portal");
                self.close_portal(&name);
            }
        }

        let _ = self.execute.write().map(|mut queue| queue.complete());
    }

    pub fn add_statement(&mut self, name: Name, statement: Statement) {
        debug!(target: CONTEXT, client_id = self.client_id, src = "add_statement",  statement = ?name);
        let _ = self
            .statements
            .write()
            .map(|mut guarded| guarded.insert(name, Arc::new(statement)));
    }

    pub fn add_portal(&mut self, name: Name, portal: Portal) {
        debug!(target: CONTEXT, client_id = self.client_id, src = "add_portal", name = ?name, portal = ?portal);
        let _ = self.portals.write().map(|mut portals| {
            portals
                .entry(name)
                .or_insert_with(Queue::new)
                .add(Arc::new(portal));
        });
    }

    pub fn get_execute(&self) -> Option<Name> {
        let queue = self.execute.read().ok()?;
        let name = queue.next()?;
        debug!(target: CONTEXT, client_id = self.client_id, src = "get_execute", name = ?name);
        Some(name.to_owned())
    }

    pub fn get_statement(&self, name: &Name) -> Option<Arc<Statement>> {
        debug!(target: CONTEXT, client_id = self.client_id, src = "get_statement", name = ?name);
        let statements = self.statements.read().ok()?;
        statements.get(name).cloned()
    }

    ///
    /// Close the portal identified by `name`
    /// Portal is removed from  queue
    ///
    pub fn close_portal(&mut self, name: &Name) {
        debug!(target: CONTEXT, client_id = self.client_id, src = "close_portal", name = ?name);
        let _ = self.portals.write().map(|mut portals| {
            portals
                .entry(name.clone())
                .and_modify(|queue| queue.complete());
        });
    }

    pub fn get_portal(&self, name: &Name) -> Option<Arc<Portal>> {
        debug!(target: CONTEXT, client_id = self.client_id, src = "get_portal");
        let portals = self.portals.read().ok()?;

        let queue = portals.get(name)?;
        queue.next().cloned()
    }

    pub fn get_portal_statement(&self, name: &Name) -> Option<Arc<Statement>> {
        debug!(target: CONTEXT, client_id = self.client_id, src = "get_portal_statement");

        let portals = self.portals.read().ok()?;
        let queue = portals.get(name)?;
        let portal = queue.next()?;

        debug!(target: CONTEXT, client_id = self.client_id, src = "get_portal_statement", portal = ?portal);

        match portal.as_ref() {
            Portal::Encrypted(p) => Some(p.statement.clone()),
            _ => None,
        }
    }

    pub fn get_statement_from_describe(&self) -> Option<Arc<Statement>> {
        let queue = self.describe.read().ok()?;
        let describe = queue.next()?;

        debug!(target: CONTEXT, client_id = self.client_id, src = "get_statement_from_describe", describe = ?describe);

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
        let name = queue.next()?;
        debug!(target: CONTEXT, client_id = self.client_id, src = "get_portal_from_execute", name = ?name);
        self.get_portal(name)
    }

    pub fn set_schema_changed(&self) {
        debug!(target: CONTEXT, client_id = self.client_id, src = "set_schema_changed");
        let _ = self.schema_changed.write().map(|mut guard| *guard = true);
    }

    pub fn schema_changed(&self) -> bool {
        self.schema_changed.read().ok().is_some_and(|s| *s)
    }

    pub fn get_table_resolver(&self) -> Arc<TableResolver> {
        self.table_resolver.clone()
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
    pub fn encrypted(statement: Arc<Statement>, format_codes: Vec<FormatCode>) -> Portal {
        Portal::Encrypted(EncryptedPortal {
            statement,
            format_codes,
        })
    }

    pub fn passthrough() -> Portal {
        Portal::Passthrough
    }
}

impl EncryptedPortal {
    // FormatCodes should not be None at this point
    // FormatCodes will be:
    //  - empty, in which case assume Text
    //  - single value, in which case use this for all columns
    //  - multiple values, in which case use the value for each column
    pub fn format_codes(&self, row_len: usize) -> Vec<FormatCode> {
        match self.format_codes.len() {
            0 => vec![FormatCode::Text; row_len],
            1 => {
                let format_code = match self.format_codes.first() {
                    Some(code) => *code,
                    None => FormatCode::Text,
                };
                vec![format_code; row_len]
            }
            _ => self.format_codes.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, Describe, Portal, Statement};
    use crate::{
        config::LogConfig,
        log,
        postgresql::messages::{describe::Target, Name},
    };
    use eql_mapper::Schema;
    use std::sync::Arc;

    fn statement() -> Statement {
        Statement {
            param_columns: vec![],
            projection_columns: vec![],
            literal_columns: vec![],
            postgres_param_types: vec![],
        }
    }

    fn portal(statement: &Arc<Statement>) -> Portal {
        Portal::encrypted(statement.clone(), vec![])
    }

    fn get_statement(portal: Arc<Portal>) -> Arc<Statement> {
        match portal.as_ref() {
            Portal::Encrypted(portal) => portal.statement.clone(),
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

        let name = Name("name".to_string());

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

        let statement_name = Name("statement".to_string());
        let portal_name = Name("portal".to_string());

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
        let statement_name_1 = Name("statement_1".to_string());
        let statement_name_2 = Name("statement_2".to_string());

        // Add statements to context
        context.add_statement(statement_name_1.clone(), statement());
        context.add_statement(statement_name_2.clone(), statement());

        // Replicate pipelined execution
        // Add multiple portals with the same name
        // Pointing to different statements
        let portal_name = Name("portal".to_string());

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

        let statement_name_1 = Name("statement_1".to_string());
        let portal_name_1 = Name::unnamed();

        let statement_name_2 = Name("statement_2".to_string());
        let portal_name_2 = Name::unnamed();

        let statement_name_3 = Name("statement_3".to_string());
        let portal_name_3 = Name("portal_3".to_string());

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
}
