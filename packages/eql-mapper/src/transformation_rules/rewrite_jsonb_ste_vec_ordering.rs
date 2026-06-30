use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{
    CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList,
    FunctionArguments, Ident, ObjectName, ObjectNamePart, ValueWithSpan,
};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::ste_vec_ordering::{
    is_eql_typed, is_ordering_operator, is_ste_vec_accessor, rhs_as_jsonb,
};
use crate::unifier::Type;
use crate::EqlMapperError;

use super::TransformationRule;

/// Rewrites ordering comparisons (`<`, `<=`, `>`, `>=`) on a jsonb STE-vec
/// element extracted via `->` / `->>` / `jsonb_path_query_first` so that the
/// comparison binds to the CLLW ORE operators in EQL 2.3.1 instead of the root
/// Block-ORE (`ob`) path.
///
/// # Why this is needed
///
/// In EQL 2.3.1 the `->` operator on `eql_v2_encrypted` returns an
/// `eql_v2.ste_vec_entry` (a single sv element carrying a CLLW ORE term `oc`).
/// A bare ordering comparison such as `(col -> selector) > $param` does not
/// resolve to the `ste_vec_entry` ordering operators because the right-hand
/// side is a root `eql_v2_encrypted` query payload (which carries a root-scope
/// `oc` term but is not a full `ste_vec_entry` and so cannot satisfy that
/// domain's CHECK constraint). Postgres instead resolves the comparison to the
/// root `eql_v2_encrypted` ordering operators which require a Block-ORE (`ob`)
/// term, raising `Expected an ore index (ob) value in json`.
///
/// EQL 2.3.1 provides `eql_v2.ore_cllw(eql_v2.ste_vec_entry)` and the companion
/// `eql_v2.ore_cllw(jsonb)` (the documented right-hand-side parameter helper)
/// which both yield an `eql_v2.ore_cllw` composite. The `ore_cllw <op>
/// ore_cllw` operators perform the CLLW ORE comparison. This rule rewrites:
///
/// ```sql
/// (col -> selector) > $param
/// ```
///
/// into:
///
/// ```sql
/// eql_v2.ore_cllw(col -> selector) > eql_v2.ore_cllw($param::jsonb)
/// ```
///
/// The right-hand side is reduced to a bare `::JSONB` value (the outer
/// `::eql_v2_encrypted` cast applied by [`super::CastParamsAsEncrypted`] /
/// [`super::CastLiteralsAsEncrypted`] is stripped) because `eql_v2.ore_cllw`
/// accepts `jsonb`, not `eql_v2_encrypted`.
#[derive(Debug)]
pub struct RewriteJsonbSteVecOrdering<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteJsonbSteVecOrdering<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    /// Wraps `expr` in a `::JSONB` cast.
    ///
    /// When `expr` is a binary operator (e.g. the `->` / `->>` accessor) it is
    /// first wrapped in parentheses so the cast applies to the whole accessor
    /// result rather than binding tighter than `->` to its right operand. The
    /// `::` cast operator binds more tightly than `->` in PostgreSQL, so
    /// `a -> b::JSONB` would parse as `a -> (b::JSONB)`.
    fn cast_jsonb(expr: Expr) -> Expr {
        let expr = match expr {
            binary @ Expr::BinaryOp { .. } => Expr::Nested(Box::new(binary)),
            other => other,
        };

        Expr::Cast {
            kind: CastKind::DoubleColon,
            expr: Box::new(expr),
            data_type: DataType::JSONB,
            format: None,
        }
    }

    /// Wraps `expr` in a call to `eql_v2.ore_cllw(...)`.
    fn wrap_ore_cllw(expr: Expr) -> Expr {
        Expr::Function(Function {
            name: ObjectName(vec![
                ObjectNamePart::Identifier(Ident::new("eql_v2")),
                ObjectNamePart::Identifier(Ident::new("ore_cllw")),
            ]),
            uses_odbc_syntax: false,
            args: FunctionArguments::List(FunctionArgumentList {
                args: vec![FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))],
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

impl<'ast> TransformationRule<'ast> for RewriteJsonbSteVecOrdering<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            let expr = target_node.downcast_mut::<Expr>().unwrap();
            if let Expr::BinaryOp { left, op: _, right } = expr {
                let dummy = Expr::Value(ValueWithSpan {
                    value: SqltkValue::Null,
                    span: Span::empty(),
                });
                let left_expr = mem::replace(&mut **left, dummy.clone());
                let right_expr = mem::replace(&mut **right, dummy);

                // The left operand is a jsonb sv-element accessor. Its result is
                // either an `eql_v2.ste_vec_entry` (`->` / `->>`) or an
                // `eql_v2_encrypted` (`jsonb_path_query_first`). Casting to
                // `jsonb` normalises both to the `eql_v2.ore_cllw(jsonb)`
                // overload, which reads the `oc` term identically in either
                // case.
                **left = Self::wrap_ore_cllw(Self::cast_jsonb(left_expr));
                **right = Self::wrap_ore_cllw(rhs_as_jsonb(right_expr));
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((Expr::BinaryOp { left, op, right },)) = node_path.last_1_as::<Expr>() {
            if is_ordering_operator(op)
                && is_ste_vec_accessor(left)
                && is_eql_typed(&self.node_types, left)
                && is_eql_typed(&self.node_types, right)
            {
                return true;
            }
        }
        false
    }
}
