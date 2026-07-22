use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{BinaryOperator, Expr, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{DomainIdentity, Type, Value};
use crate::EqlMapperError;

use super::helpers::{eql_v3_term_call, is_comparison_op};
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

    /// The term function for `op` on a column with `identity`, or `None` if the
    /// domain provides no term for that operator.
    fn term_fn_for(op: &BinaryOperator, identity: &DomainIdentity) -> Option<&'static str> {
        match op {
            BinaryOperator::Eq | BinaryOperator::NotEq => identity.eq_term_fn(),
            BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq => identity.ord_term_fn(),
            _ => None,
        }
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
        if !is_comparison_op(op) {
            return Ok(false);
        }
        let Some(identity) = self
            .eql_identity_of(left)
            .or_else(|| self.eql_identity_of(right))
        else {
            return Ok(false);
        };

        let Some(term_fn) = Self::term_fn_for(op, &identity) else {
            return Err(EqlMapperError::Transform(format!(
                "encrypted column {} does not support operator {op} (domain {})",
                identity.token, identity.domain.value
            )));
        };

        if let Expr::BinaryOp { left, right, .. } = target_node.downcast_mut::<Expr>().unwrap() {
            let dummy = Expr::Value(ValueWithSpan {
                value: SqltkValue::Null,
                span: Span::empty(),
            });
            let left_expr = mem::replace(&mut **left, dummy.clone());
            let right_expr = mem::replace(&mut **right, dummy);
            **left = eql_v3_term_call(term_fn, left_expr);
            **right = eql_v3_term_call(term_fn, right_expr);
            return Ok(true);
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((Expr::BinaryOp { left, op, right },)) = node_path.last_1_as::<Expr>() {
            if is_comparison_op(op) {
                return self.eql_identity_of(left).is_some()
                    || self.eql_identity_of(right).is_some();
            }
        }
        false
    }
}
