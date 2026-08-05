use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{Expr, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{DomainIdentity, EqlTerm, Type, Value};
use crate::EqlMapperError;

use super::helpers::{
    cast_encrypted_operand, eql_v3_term_call, is_comparison_op, query_operand_domain, term_fn_for,
};
use super::TransformationRule;

/// Rewrites scalar comparison operators on encrypted columns into the EQL v3
/// functional-index form (ADR-0001, ADR-0003):
///
/// - `col = x`  → `eql_v3.eq_term(col) = eql_v3.eq_term(x)` (or `ord_term` when
///   the domain stores no `hm`)
/// - `col > x`  → `eql_v3.ord_term(col) > eql_v3.ord_term(x)` (`ord_term_ore` for
///   block-ORE domains)
///
/// The term function is chosen from the column's domain identity; a column whose
/// domain provides no term for the operator is a capability error (this is the
/// same absence the type checker's bound check raises on — this rule is the
/// backstop at rewrite time).
///
/// Operands are moved with `mem::replace` (not cloned) so their `NodeKey`
/// identity survives for the cast rules. Post-order traversal means the operand
/// literals/params have already been cast to their v3 domains by the time this
/// rule wraps them.
#[derive(Debug)]
pub struct RewriteEqlComparisonOps<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlComparisonOps<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    fn eql_identity_of(&self, expr: &'ast Expr) -> Option<DomainIdentity> {
        match self.node_types.get(&NodeKey::new(expr)) {
            Some(Type::Value(Value::Eql(eql_term))) => {
                Some(eql_term.eql_value().domain_identity().clone())
            }
            _ => None,
        }
    }

    /// Encrypted JSON field equality is value-selector containment, rewritten by
    /// [`super::RewriteJsonValueSelectorEq`], not a term comparison. `eq_term`
    /// has no unique overload for a JSON query operand, so wrapping one here
    /// would produce SQL PostgreSQL rejects.
    fn is_json_value_selector_eq(&self, left: &'ast Expr, right: &'ast Expr) -> bool {
        [left, right].into_iter().any(|expr| {
            matches!(
                self.node_types.get(&NodeKey::new(expr)),
                Some(Type::Value(Value::Eql(EqlTerm::JsonValueSelector(_))))
            )
        })
    }

}

impl<'ast> TransformationRule<'ast> for RewriteEqlComparisonOps<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if !self.would_edit(node_path, target_node) {
            return Ok(false);
        }

        // Read the operator and the encrypted operand's domain identity from the
        // ORIGINAL nodes (node_types is keyed by them); `target_node`'s children
        // may already be rebuilt with different NodeKeys.
        let Some((Expr::BinaryOp { left, op, right },)) = node_path.last_1_as::<Expr>() else {
            return Ok(false);
        };
        if !is_comparison_op(op) || self.is_json_value_selector_eq(left, right) {
            return Ok(false);
        }
        let Some(identity) = self
            .eql_identity_of(left)
            .or_else(|| self.eql_identity_of(right))
        else {
            return Ok(false);
        };

        let Some(term_fn) = term_fn_for(op, &identity) else {
            return Err(EqlMapperError::Transform(format!(
                "encrypted column {} does not support operator {op} (domain {})",
                identity.token, identity.domain.value
            )));
        };

        if let Expr::BinaryOp {
            left: target_left,
            right: target_right,
            ..
        } = target_node.downcast_mut::<Expr>().unwrap()
        {
            // Cast the operands before wrapping them: this rule owns the
            // comparison, so it knows both are query operands and casts them to
            // the term-only `eql_v3.query_*` twin.
            cast_encrypted_operand(&self.node_types, left, target_left, query_operand_domain);
            cast_encrypted_operand(&self.node_types, right, target_right, query_operand_domain);

            let dummy = Expr::Value(ValueWithSpan {
                value: SqltkValue::Null,
                span: Span::empty(),
            });
            let left_expr = mem::replace(&mut **target_left, dummy.clone());
            let right_expr = mem::replace(&mut **target_right, dummy);
            **target_left = eql_v3_term_call(term_fn, left_expr);
            **target_right = eql_v3_term_call(term_fn, right_expr);
            return Ok(true);
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((Expr::BinaryOp { left, op, right },)) = node_path.last_1_as::<Expr>() {
            if is_comparison_op(op) && !self.is_json_value_selector_eq(left, right) {
                return self.eql_identity_of(left).is_some()
                    || self.eql_identity_of(right).is_some();
            }
        }
        false
    }
}
