use eql_mapper::{Projection, Value};
use sqlparser::ast;
use std::collections::HashMap;

use super::messages::Destination;

#[derive(Debug, Clone)]
pub struct Statement {
    /// A SQL statement. This will have been transformed if the statement received by the front-end
    /// required type-checking and it was transformed to perform EQL conversion.
    ast: ast::Statement,
    postgres_param_types: Vec<i32>,
    /// If this was a type-checked statement, then `eql_metadata` will be `Some(_)`, else `None`.
    eql_metadata: Option<EqlMetadata>,
}

/// Metadata for a [`ast::Statement`] type that could only be safely handled after processing by [`eql_mapper`].
#[derive(Debug, Clone)]
pub struct EqlMetadata {
    eql_param_types: Vec<Value>,
    eql_resultset_type: Option<Projection>,
}

impl Statement {
    pub fn new_unmapped(ast: ast::Statement, postgres_param_types: Vec<i32>) -> Statement {
        Statement {
            ast,
            postgres_param_types,
            eql_metadata: None,
        }
    }

    pub fn new_mapped(
        ast: ast::Statement,
        postgres_param_types: Vec<i32>,
        eql_param_types: Vec<Value>,
        eql_resultset_type: Option<Projection>,
    ) -> Statement {
        Statement {
            ast,
            postgres_param_types,
            eql_metadata: Some(EqlMetadata {
                eql_param_types,
                eql_resultset_type,
            }),
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
