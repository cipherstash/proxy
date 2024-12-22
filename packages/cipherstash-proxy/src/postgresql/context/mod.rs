use arc_swap::ArcSwapOption;
use eql_mapper::{Projection, Value};
use sqlparser::ast;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use super::messages::{describe::Describe, Destination};

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
    pub param_type_mapping: Vec<Option<postgres_types::Type>>,
    /// If this was a type-checked statement, then `eql_metadata` will be `Some(_)`, else `None`.
    metadata: Option<Metadata>,
}

/// Metadata for a [`ast::Statement`] type that could only be safely handled after processing by [`eql_mapper`].
#[derive(Debug, Clone)]
pub struct Metadata {
    param_types: Vec<Value>,
    result_projection: Option<Projection>,
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

    pub fn add(&mut self, key: Destination, statement: Statement) {
        let mut lock = self.statements.write().unwrap();
        lock.insert(key, statement);
    }

    // TODO Remove cloning
    // We can probably do work INSIDE the context
    pub fn get(&self, key: &Destination) -> Option<Statement> {
        let lock = self.statements.read().unwrap();
        lock.get(key).cloned()
    }

    pub fn get_param_types(&self, key: &Destination) -> Option<Vec<Option<postgres_types::Type>>> {
        let lock = self.statements.read().unwrap();
        match lock.get(key) {
            Some(statement) => Some(statement.param_type_mapping.clone()),
            None => None,
        }
    }

    pub fn remove(&mut self, key: &Destination) -> Option<Statement> {
        let mut lock = self.statements.write().unwrap();
        lock.remove(key)
    }
}

impl Statement {
    pub fn unmapped(ast: ast::Statement, postgres_param_types: Vec<i32>) -> Statement {
        Statement {
            ast,
            postgres_param_types,
            metadata: None,
            param_type_mapping: vec![],
        }
    }

    pub fn mapped(
        ast: ast::Statement,
        postgres_param_types: Vec<i32>,
        param_type_mapping: Vec<Option<postgres_types::Type>>,
        eql_param_types: Vec<Value>,
        eql_resultset_type: Option<Projection>,
    ) -> Statement {
        Statement {
            ast,
            postgres_param_types,
            param_type_mapping,
            metadata: Some(Metadata {
                param_types: eql_param_types,
                result_projection: eql_resultset_type,
            }),
        }
    }
}
