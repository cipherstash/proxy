use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{
    BinaryOperator, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList,
    FunctionArguments, Ident, ObjectName, ObjectNamePart, ValueWithSpan,
};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{EqlTerm, Type, Value};
use crate::EqlMapperError;

use super::helpers::{cast_encrypted_operand, query_operand_domain};
use super::TransformationRule;

/// Rewrites equality on an encrypted JSON **field** into value-selector
/// containment:
///
/// - `col -> sel  = value` → `eql_v3.jsonb_contains(col, <needle>)`
/// - `col ->> sel = value` → same
/// - `jsonb_path_query_first(col, sel) = value` → same
/// - `<>` negates: `NOT eql_v3.jsonb_contains(col, <needle>)`
///
/// where `<needle>` is the value operand, already cast to `eql_v3.query_json`
/// by the cast rules. Exact JSON equality in EQL v3 is selector containment: the
/// needle is one keyed MAC over the path and the canonicalised value together
/// (`QueryOp::SteVecValueSelector`), which the proxy composes from the two SQL
/// operands. So this rule **discards** the field access: `col` is lifted out and
/// the selector operand disappears from the statement.
///
/// A discarded selector *placeholder* stays declared in Parse but unreferenced
/// in the SQL, which PostgreSQL permits as long as its type is known. That keeps
/// input and output param numbering identical, so Bind stays positional.
///
/// The generic `eq_term` wrap ([`super::RewriteEqlComparisonOps`]) must not also
/// fire here — `eql_v3.eq_term` has no unique overload for a JSON query operand,
/// and containment is not a term comparison. That rule skips any comparison
/// whose operand is [`EqlTerm::JsonValueSelector`].
#[derive(Debug)]
pub struct RewriteJsonValueSelectorEq<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteJsonValueSelectorEq<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    /// Whether `expr` is the fused value operand of a JSON field equality.
    fn is_value_selector(&self, expr: &'ast Expr) -> bool {
        matches!(
            self.node_types.get(&NodeKey::new(expr)),
            Some(Type::Value(Value::Eql(EqlTerm::JsonValueSelector(_))))
        )
    }

    /// The container expression of a JSON field access — the `col` of
    /// `col -> sel` or of `jsonb_path_query_first(col, sel)`.
    ///
    /// Read from the ORIGINAL AST (via `node_path`), because by the time this
    /// rule runs the field access has already been rewritten to
    /// `eql_v3."->"(col, sel)` by [`super::RewriteContainmentOps`].
    fn container_of(expr: &Expr) -> Option<&Expr> {
        match expr {
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Arrow | BinaryOperator::LongArrow,
                ..
            } => Some(&**left),

            Expr::Function(function) => match &function.args {
                FunctionArguments::List(list) => match list.args.as_slice() {
                    [FunctionArg::Unnamed(FunctionArgExpr::Expr(container)), _] => Some(container),
                    _ => None,
                },
                _ => None,
            },

            _ => None,
        }
    }

    /// Splits a comparison into `(container, value operand is on the right)`, or
    /// `None` if this is not a JSON field equality.
    fn match_comparison(
        &self,
        left: &'ast Expr,
        op: &BinaryOperator,
        right: &'ast Expr,
    ) -> Option<(&'ast Expr, bool)> {
        if !matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq) {
            return None;
        }

        if self.is_value_selector(right) {
            Self::container_of(left).map(|container| (container, true))
        } else if self.is_value_selector(left) {
            Self::container_of(right).map(|container| (container, false))
        } else {
            None
        }
    }

    fn jsonb_contains(container: Expr, needle: Expr) -> Expr {
        Expr::Function(Function {
            name: ObjectName(vec![
                ObjectNamePart::Identifier(Ident::new("eql_v3")),
                ObjectNamePart::Identifier(Ident::new("jsonb_contains")),
            ]),
            uses_odbc_syntax: false,
            args: FunctionArguments::List(FunctionArgumentList {
                args: vec![
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(container)),
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(needle)),
                ],
                duplicate_treatment: None,
                clauses: vec![],
            }),
            parameters: FunctionArguments::None,
            filter: None,
            null_treatment: None,
            over: None,
            within_group: vec![],
        })
    }
}

impl<'ast> TransformationRule<'ast> for RewriteJsonValueSelectorEq<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Match against the ORIGINAL nodes: `node_types` is keyed by them, and
        // `target_node`'s children have already been rebuilt by earlier rules.
        let Some((Expr::BinaryOp { left, op, right },)) = node_path.last_1_as::<Expr>() else {
            return Ok(false);
        };

        let Some((container, value_on_right)) = self.match_comparison(left, op, right) else {
            return Ok(false);
        };

        let negated = matches!(op, BinaryOperator::NotEq);

        let Some(expr) = target_node.downcast_mut::<Expr>() else {
            return Ok(false);
        };
        let Expr::BinaryOp {
            left: target_left,
            right: target_right,
            ..
        } = expr
        else {
            return Ok(false);
        };

        // Move (not clone) the transformed value operand so its NodeKey identity
        // survives; the container comes from the original AST, where it is still
        // the bare column reference the containment call needs.
        // The needle is a query operand of this rule's own predicate: cast it to
        // `eql_v3.query_json`, the containment-needle domain.
        let (original_needle, target_needle) = if value_on_right {
            (right, target_right)
        } else {
            (left, target_left)
        };
        cast_encrypted_operand(
            &self.node_types,
            original_needle,
            target_needle,
            query_operand_domain,
        );

        let dummy = Expr::Value(ValueWithSpan {
            value: SqltkValue::Null,
            span: Span::empty(),
        });
        let needle = mem::replace(&mut **target_needle, dummy);

        let contains = Self::jsonb_contains(container.clone(), needle);

        *expr = if negated {
            Expr::UnaryOp {
                op: sqltk::parser::ast::UnaryOperator::Not,
                expr: Box::new(Expr::Nested(Box::new(contains))),
            }
        } else {
            contains
        };

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        match node_path.last_1_as::<Expr>() {
            Some((Expr::BinaryOp { left, op, right },)) => {
                self.match_comparison(left, op, right).is_some()
            }
            _ => false,
        }
    }
}
