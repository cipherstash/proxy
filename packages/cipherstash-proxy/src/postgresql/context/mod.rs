use super::{
    format_code::FormatCode,
    messages::{describe::Describe, execute::Execute, Destination},
};
use crate::Identifier;
use arc_swap::ArcSwapOption;
use cipherstash_config::ColumnConfig;
use postgres_types::Type;
use sqlparser::ast;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

#[derive(Debug, Clone)]
pub struct Context {
    pub statements: Arc<RwLock<HashMap<Destination, Statement>>>,
    pub portals: Arc<RwLock<HashMap<Destination, Portal>>>,
    pub describe: Arc<ArcSwapOption<Describe>>,
    pub execute: Arc<ArcSwapOption<Execute>>,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub identifier: Identifier,
    pub config: ColumnConfig,
    pub postgres_type: Type,
}

#[derive(Debug, Clone)]
pub struct Statement {
    /// A SQL statement. This will have been transformed if the statement received by the front-end
    /// required type-checking and it was transformed to perform EQL conversion.
    ast: ast::Statement,
    pub postgres_param_types: Vec<i32>,
    pub param_columns: Vec<Option<Column>>,
    pub projection_columns: Vec<Option<Column>>,
}

#[derive(Debug, Clone)]
pub struct Portal {
    pub format_codes: Vec<FormatCode>,
    pub statement: Statement,
}

impl Context {
    pub fn new() -> Context {
        Context {
            statements: Arc::new(RwLock::new(HashMap::new())),
            portals: Arc::new(RwLock::new(HashMap::new())),
            describe: Arc::new(ArcSwapOption::from(None)),
            execute: Arc::new(ArcSwapOption::from(None)),
        }
    }

    pub fn describe(&mut self, describe: Describe) {
        self.describe.swap(Some(Arc::new(describe)));
    }

    pub fn describe_complete(&mut self) {
        self.describe.swap(None);
    }

    pub fn execute(&mut self, execute: Execute) {
        self.execute.swap(Some(Arc::new(execute)));
    }

    pub fn execute_complete(&mut self) {
        self.execute.swap(None);
    }

    pub fn add(&mut self, key: Destination, statement: Statement) {
        let mut statment_write = self.statements.write().unwrap();
        statment_write.insert(key, statement);
    }

    pub fn add_portal(&mut self, key: Destination, portal: Portal) {
        let mut portal_write = self.portals.write().unwrap();
        portal_write.insert(key, portal);
    }

    // TODO Remove cloning
    // We can probably do work INSIDE the context
    pub fn get(&self, key: &Destination) -> Option<Statement> {
        let statment_read = self.statements.read().unwrap();
        statment_read.get(key).cloned()
    }

    pub fn get_param_columns_for_describe(&self) -> Option<Vec<Option<Column>>> {
        let guard = self.describe.load();
        match guard.as_ref() {
            Some(describe) => self.get_param_columns(&describe.name),
            None => None,
        }
    }

    pub fn get_projection_columns_for_describe(&self) -> Option<Vec<Option<Column>>> {
        let guard = self.describe.load();
        match guard.as_ref() {
            Some(describe) => self.get_projection_columns(&describe.name),
            None => None,
        }
    }

    pub fn get_param_columns(&self, key: &Destination) -> Option<Vec<Option<Column>>> {
        let statement_read = self.statements.read().unwrap();
        statement_read
            .get(key)
            .map(|statement| statement.param_columns.clone())
    }

    pub fn get_projection_columns(&self, key: &Destination) -> Option<Vec<Option<Column>>> {
        let statement_read = self.statements.read().unwrap();
        statement_read
            .get(key)
            .map(|statement| statement.projection_columns.clone())
    }

    pub fn get_result_format_codes_for_execute(&self) -> Option<Vec<FormatCode>> {
        let guard = self.execute.load();
        match guard.as_ref() {
            Some(execute) => self.get_result_format_codes_for_portal(&execute.portal),
            None => None,
        }
    }

    pub fn get_result_format_codes_for_portal(&self, key: &Destination) -> Option<Vec<FormatCode>> {
        let portal_read = self.portals.read().unwrap();
        portal_read
            .get(key)
            .map(|portal| portal.format_codes.clone())
    }

    pub fn remove(&mut self, key: &Destination) -> Option<Statement> {
        let mut statment_write = self.statements.write().unwrap();
        statment_write.remove(key)
    }
}

impl Statement {
    pub fn mapped(
        ast: ast::Statement,
        postgres_param_types: Vec<i32>,
        param_columns: Vec<Option<Column>>,
        projection_columns: Vec<Option<Column>>,
    ) -> Statement {
        Statement {
            ast,
            postgres_param_types,
            param_columns,
            projection_columns,
        }
    }
}

impl Portal {
    pub fn new(statement: Statement, format_codes: Vec<FormatCode>) -> Portal {
        Portal {
            statement,
            format_codes,
        }
    }
}
