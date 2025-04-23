use std::{collections::HashMap, sync::Arc};

use sqltk::{AsNodeKey, NodeKey, Transform, Transformable};
use sqltk_parser::ast::{self, Statement};

use crate::{EncryptedStatement, EqlMapperError, EqlValue, Param, Projection, Type, Value};

/// The result returned from a successful call to [`type_check`].
#[derive(Debug)]
pub struct TypedStatement<'ast> {
    /// The SQL statement which was type-checked against the schema.
    pub statement: &'ast Statement,

    /// The return type of the statement
    pub projection: Projection,

    /// The types of all params discovered from [`Value::Placeholder`] nodes in the SQL statement.
    pub params: Vec<(Param, Value)>,

    /// The types and values of all literals from the SQL statement.
    pub literals: Vec<(EqlValue, &'ast ast::Value)>,

    pub node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> TypedStatement<'ast> {
    /// Transforms the SQL statement by replacing all plaintext literals with EQL equivalents.
    pub fn transform(
        &self,
        encrypted_literals: HashMap<NodeKey<'ast>, sqltk_parser::ast::Value>,
    ) -> Result<Statement, EqlMapperError> {
        for (key, _) in encrypted_literals.iter() {
            if !self
                .literals
                .iter()
                .any(|(_, node)| &node.as_node_key() == key)
            {
                return Err(EqlMapperError::Transform(String::from("encrypted literals refers to a literal node which is not present in the SQL statement")));
            }
        }

        let mut transformer =
            EncryptedStatement::new(encrypted_literals, Arc::clone(&self.node_types));

        let statement = self.statement.apply_transform(&mut transformer)?;
        transformer.check_postcondition()?;
        Ok(statement)
    }

    pub fn literal_values(&self) -> Vec<&sqltk_parser::ast::Value> {
        if self.literals.is_empty() {
            return vec![];
        }

        self.literals
            .iter()
            .map(|(_eql_value, ast_value)| *ast_value)
            .collect::<Vec<_>>()
    }
}
