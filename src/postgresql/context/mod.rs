use sqlparser::ast;
use std::collections::HashMap;

use super::messages::Destination;

#[derive(Debug, Clone)]
pub struct Statement {
    ast: ast::Statement,
    param_types: Vec<i32>,
}

impl Statement {
    pub fn new(ast: ast::Statement, param_types: Vec<i32>) -> Statement {
        Statement { ast, param_types }
    }
}

#[derive(Debug, Clone)]
pub struct Context {
    statements: HashMap<String, Statement>,
}

impl Context {
    pub fn new() -> Context {
        Context {
            statements: HashMap::new(),
        }
    }

    pub fn add(&mut self, name: &Destination, statement: Statement) {
        let name = name.as_str().to_owned();
        self.statements.insert(name, statement);
    }

    pub fn get(&mut self, name: &Destination) -> Option<&Statement> {
        let name = name.as_str();
        self.statements.get(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<Statement> {
        self.statements.remove(name)
    }
}
