use super::messages::{describe::Describe, Destination};
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
    // pub statements: HashMap<Destination, Statement>,
    pub describe: Arc<ArcSwapOption<Describe>>,
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

impl Context {
    pub fn new() -> Context {
        Context {
            statements: Arc::new(RwLock::new(HashMap::new())),
            // statements: HashMap::new(),
            describe: Arc::new(ArcSwapOption::from(None)),
        }
    }

    pub fn describe(&mut self, describe: Describe) {
        self.describe.swap(Some(Arc::new(describe)));
    }

    pub fn describe_complete(&mut self) {
        self.describe.swap(None);
    }

    pub fn add(&mut self, key: Destination, statement: Statement) {
        let mut statment_write = self.statements.write().unwrap();
        statment_write.insert(key, statement);
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
        let statment_read = self.statements.read().unwrap();
        match statment_read.get(key) {
            Some(statement) => Some(statement.param_columns.clone()),
            None => None,
        }
    }

    pub fn get_projection_columns(&self, key: &Destination) -> Option<Vec<Option<Column>>> {
        let statment_read = self.statements.read().unwrap();
        match statment_read.get(key) {
            Some(statement) => Some(statement.projection_columns.clone()),
            None => None,
        }
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
