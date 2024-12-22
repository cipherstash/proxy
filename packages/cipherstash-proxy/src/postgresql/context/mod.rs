use arc_swap::ArcSwapOption;
use sqlparser::ast;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use super::messages::{
    describe::{self, Describe},
    Destination,
};

#[derive(Debug, Clone)]
pub struct Context {
    pub statements: Arc<RwLock<HashMap<Destination, Statement>>>,
    pub describe: Arc<ArcSwapOption<Describe>>,
}

#[derive(Debug, Clone)]
pub struct Statement {
    /// A SQL statement. This will have been transformed if the statement received by the front-end
    /// required type-checking and it was transformed to perform EQL conversion.
    ast: ast::Statement,
    pub postgres_param_types: Vec<i32>,
    pub param_types: Vec<Option<postgres_types::Type>>,
    pub projection_types: Vec<Option<postgres_types::Type>>,
}

impl Context {
    pub fn new() -> Context {
        Context {
            statements: Arc::new(RwLock::new(HashMap::new())),
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

    pub fn get_param_types_for_describe(&self) -> Option<Vec<Option<postgres_types::Type>>> {
        let guard = self.describe.load();
        match guard.as_ref() {
            Some(describe) => self.get_param_types(&describe.name),
            None => None,
        }
    }
    pub fn get_projection_types_for_describe(&self) -> Option<Vec<Option<postgres_types::Type>>> {
        let guard = self.describe.load();
        match guard.as_ref() {
            Some(describe) => self.get_projection_types(&describe.name),
            None => None,
        }
    }

    pub fn get_param_types(&self, key: &Destination) -> Option<Vec<Option<postgres_types::Type>>> {
        let statment_read = self.statements.read().unwrap();
        match statment_read.get(key) {
            Some(statement) => Some(statement.param_types.clone()),
            None => None,
        }
    }

    pub fn get_projection_types(
        &self,
        key: &Destination,
    ) -> Option<Vec<Option<postgres_types::Type>>> {
        let statment_read = self.statements.read().unwrap();
        match statment_read.get(key) {
            Some(statement) => Some(statement.projection_types.clone()),
            None => None,
        }
    }

    pub fn remove(&mut self, key: &Destination) -> Option<Statement> {
        let mut statment_write = self.statements.write().unwrap();
        statment_write.remove(key)
    }
}

impl Statement {
    pub fn unmapped(ast: ast::Statement, postgres_param_types: Vec<i32>) -> Statement {
        Statement {
            ast,
            postgres_param_types,
            param_types: vec![],
            projection_types: vec![],
        }
    }

    pub fn mapped(
        ast: ast::Statement,
        postgres_param_types: Vec<i32>,
        param_types: Vec<Option<postgres_types::Type>>,
        projection_types: Vec<Option<postgres_types::Type>>,
    ) -> Statement {
        Statement {
            ast,
            postgres_param_types,
            param_types,
            projection_types,
        }
    }
}
