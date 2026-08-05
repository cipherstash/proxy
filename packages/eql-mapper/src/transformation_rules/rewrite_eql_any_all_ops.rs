use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{Array, BinaryOperator, Expr, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{DomainIdentity, Type, Value};
use crate::EqlMapperError;

use super::helpers::{
    cast_encrypted_operand, eql_v3_term_call, is_comparison_op, query_operand_domain, term_fn_for,
};
use super::TransformationRule;

/// Rewrites `ANY`/`ALL` comparisons on encrypted operands into the EQL v3
/// functional-index form, elementwise over the array:
///
/// - `col = ANY(ARRAY['a', 'b'])` →
///   `eql_v3.eq_term(col) = ANY(ARRAY[eql_v3.eq_term('…'::JSONB::eql_v3.query_…), …])`
///
/// The quantifier distributes the operator over the array's elements, so the
/// scalar comparison rewrite ([`super::RewriteEqlComparisonOps`]) applies to
/// each element in turn: the same term function on both sides, chosen from the
/// encrypted operand's domain identity by the comparison operator.
///
/// Only the ARRAY-literal shape reaches this rule: the type checker refuses an
/// encrypted subquery projection or bare array param, which have no elements to
/// rewrite (see `InferType<Expr>` for `AnyOp`/`AllOp`).
#[derive(Debug)]
pub struct RewriteEqlAnyAllOps<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlAnyAllOps<'ast> {
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

    /// The operands of the original node, when it is an ANY/ALL comparison:
    /// the scalar side, the operator, and the array elements.
    fn original_operands(
        node_path: &NodePath<'ast>,
    ) -> Option<(&'ast Expr, &'ast BinaryOperator, &'ast [Expr])> {
        let (expr,) = node_path.last_1_as::<Expr>()?;

        let (left, compare_op, right) = match expr {
            Expr::AnyOp {
                left,
                compare_op,
                right,
                ..
            }
            | Expr::AllOp {
                left,
                compare_op,
                right,
            } => (&**left, compare_op, &**right),
            _ => return None,
        };

        let Expr::Array(Array { elem, .. }) = right else {
            return None;
        };

        Some((left, compare_op, elem.as_slice()))
    }
}

impl<'ast> TransformationRule<'ast> for RewriteEqlAnyAllOps<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if !self.would_edit(node_path, target_node) {
            return Ok(false);
        }

        // Read the operator and the encrypted operand's domain identity from
        // the ORIGINAL nodes (node_types is keyed by them); `target_node`'s
        // children may already be rebuilt with different NodeKeys.
        let Some((left, compare_op, elements)) = Self::original_operands(node_path) else {
            return Ok(false);
        };

        let Some(identity) = self
            .eql_identity_of(left)
            .or_else(|| elements.iter().find_map(|elem| self.eql_identity_of(elem)))
        else {
            return Ok(false);
        };

        let Some(term_fn) = term_fn_for(compare_op, &identity) else {
            return Err(EqlMapperError::Transform(format!(
                "encrypted column {} does not support operator {compare_op} (domain {})",
                identity.token, identity.domain.value
            )));
        };

        let (Expr::AnyOp {
            left: target_left,
            right: target_right,
            ..
        }
        | Expr::AllOp {
            left: target_left,
            right: target_right,
            ..
        }) = target_node.downcast_mut::<Expr>().unwrap()
        else {
            return Ok(false);
        };

        let Expr::Array(Array {
            elem: target_elements,
            ..
        }) = &mut **target_right
        else {
            // The type checker only admits the ARRAY-literal shape for
            // encrypted operands, so a non-array here is an invariant break.
            return Err(EqlMapperError::Transform(
                "ANY/ALL on an encrypted operand without an ARRAY literal".into(),
            ));
        };

        let dummy = Expr::Value(ValueWithSpan {
            value: SqltkValue::Null,
            span: Span::empty(),
        });

        // Cast each operand before wrapping it: this rule owns the comparison,
        // so it knows every operand is a query operand and casts it to the
        // term-only `eql_v3.query_*` twin.
        cast_encrypted_operand(&self.node_types, left, target_left, query_operand_domain);
        let left_expr = mem::replace(&mut **target_left, dummy.clone());
        **target_left = eql_v3_term_call(term_fn, left_expr);

        for (original, target) in elements.iter().zip(target_elements.iter_mut()) {
            cast_encrypted_operand(&self.node_types, original, target, query_operand_domain);
            let elem_expr = mem::replace(target, dummy.clone());
            *target = eql_v3_term_call(term_fn, elem_expr);
        }

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((left, compare_op, elements)) = Self::original_operands(node_path) {
            if is_comparison_op(compare_op) {
                return self.eql_identity_of(left).is_some()
                    || elements
                        .iter()
                        .any(|elem| self.eql_identity_of(elem).is_some());
            }
        }
        false
    }
}
