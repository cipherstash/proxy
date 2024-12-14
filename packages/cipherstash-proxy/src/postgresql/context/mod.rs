use eql_mapper::{Projection, Value};
use sqlparser::ast;
use std::collections::HashMap;

use super::messages::Destination;

#[derive(Debug, Clone)]
pub struct Statement {
    ast: ast::Statement,
    postgres_param_types: Vec<i32>,
    eql_param_types: Vec<Value>,
    eql_resultset_type: Option<Projection>,
}

impl Statement {
    pub fn new(
        ast: ast::Statement,
        postgres_param_types: Vec<i32>,
        eql_param_types: Vec<Value>,
        eql_resultset_type: Option<Projection>,
    ) -> Statement {
        Statement {
            ast,
            postgres_param_types,
            eql_param_types,
            eql_resultset_type,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Context {
    statements: HashMap<Destination, Statement>,
}

impl Context {
    pub fn new() -> Context {
        Context {
            statements: HashMap::new(),
        }
    }

    pub fn add(&mut self, key: Destination, statement: Statement) {
        self.statements.insert(key, statement);
    }

    pub fn get(&mut self, key: &Destination) -> Option<&Statement> {
        self.statements.get(key)
    }

    pub fn remove(&mut self, key: &Destination) -> Option<Statement> {
        self.statements.remove(key)
    }
}
