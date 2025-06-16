use std::{any::type_name, collections::HashMap};

use sqltk::parser::ast::{Expr, Value, ValueWithSpan};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::EqlMapperError;

use super::helpers::cast_as_encrypted;
use super::TransformationRule;

#[derive(Debug)]
pub struct CastLiteralsAsEncrypted<'ast> {
    encrypted_literals: HashMap<NodeKey<'ast>, Value>,
}

impl<'ast> CastLiteralsAsEncrypted<'ast> {
    pub fn new(encrypted_literals: HashMap<NodeKey<'ast>, Value>) -> Self {
        Self { encrypted_literals }
    }
}

impl<'ast> TransformationRule<'ast> for CastLiteralsAsEncrypted<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((Expr::Value(ValueWithSpan { value, .. }),)) = node_path.last_1_as::<Expr>()
            {
                if let Some(replacement) = self.encrypted_literals.remove(&NodeKey::new(value)) {
                    let target_node = target_node.downcast_mut::<Expr>().unwrap();
                    *target_node = cast_as_encrypted(replacement);
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((Expr::Value(ValueWithSpan { value, .. }),)) = node_path.last_1_as::<Expr>() {
            return self.encrypted_literals.contains_key(&NodeKey::new(value));
        }
        false
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
