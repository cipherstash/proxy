use std::{any::type_name, collections::HashMap};

use sqltk::parser::ast::Value;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::EqlMapperError;

use super::TransformationRule;

#[derive(Debug)]
pub struct ReplacePlaintextEqlLiterals<'ast> {
    encrypted_literals: HashMap<NodeKey<'ast>, Value>,
}

impl<'ast> ReplacePlaintextEqlLiterals<'ast> {
    pub fn new(encrypted_literals: HashMap<NodeKey<'ast>, Value>) -> Self {
        Self { encrypted_literals }
    }
}

impl<'ast> TransformationRule<'ast> for ReplacePlaintextEqlLiterals<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((value,)) = node_path.last_1_as::<Value>() {
                if let Some(replacement) = self.encrypted_literals.remove(&NodeKey::new(value)) {
                    let target_node = target_node.downcast_mut::<Value>().unwrap();
                    *target_node = replacement;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((value,)) = node_path.last_1_as::<Value>() {
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
