use std::{any::type_name, collections::HashMap};

use sqlparser::ast::Expr;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::EqlMapperError;

use super::TransformationRule;

#[derive(Debug)]
pub struct ReplacePlaintextEqlLiterals<'ast> {
    encrypted_literals: HashMap<NodeKey<'ast>, Expr>,
}

impl<'ast> ReplacePlaintextEqlLiterals<'ast> {
    pub fn new(encrypted_literals: HashMap<NodeKey<'ast>, Expr>) -> Self {
        Self { encrypted_literals }
    }
}

impl<'ast> TransformationRule<'ast> for ReplacePlaintextEqlLiterals<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<(), EqlMapperError> {
        if let Some((expr,)) = node_path.last_1_as::<Expr>() {
            if let Some(replacement) = self.encrypted_literals.remove(&NodeKey::new(expr)) {
                let target_node = target_node.downcast_mut::<Expr>().unwrap();
                *target_node = replacement;
            }
        }

        Ok(())
    }

    fn check_postcondition(&self) -> Result<(), EqlMapperError> {
        if self.encrypted_literals.is_empty() {
            Ok(())
        } else {
            Err(EqlMapperError::Transform(format!(
                "Postcondition failed in {}: unused encrypted literals",
                type_name::<Self>()
            )))
        }
    }
}
