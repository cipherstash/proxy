use std::{any::type_name, collections::HashMap};

use sqltk::parser::ast::{Expr, Value, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::EqlMapperError;

use super::TransformationRule;

/// Replaces each plaintext literal with the encrypted value the proxy produced
/// for it.
///
/// Substitution only — **no casting**. Where a literal appears determines the
/// domain it must be cast to, and that is known with certainty only by the rule
/// that owns the surrounding construct (a comparison, a containment, an
/// `INSERT`). Those rules apply the cast; this one just puts the ciphertext in
/// place, wherever it is.
///
/// Runs before the casting rules in the tuple so the value is already in place
/// by the time a rule wraps it.
#[derive(Debug)]
pub struct SubstituteEncryptedLiterals<'ast> {
    encrypted_literals: HashMap<NodeKey<'ast>, Value>,
}

impl<'ast> SubstituteEncryptedLiterals<'ast> {
    pub fn new(encrypted_literals: HashMap<NodeKey<'ast>, Value>) -> Self {
        Self { encrypted_literals }
    }
}

impl<'ast> TransformationRule<'ast> for SubstituteEncryptedLiterals<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        let Some((Expr::Value(ValueWithSpan { value, .. }),)) = node_path.last_1_as::<Expr>()
        else {
            return Ok(false);
        };

        let Some(replacement) = self.encrypted_literals.remove(&NodeKey::new(value)) else {
            return Ok(false);
        };

        let target_node = target_node.downcast_mut::<Expr>().unwrap();
        *target_node = Expr::Value(ValueWithSpan {
            value: replacement,
            span: Span::empty(),
        });

        Ok(true)
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
