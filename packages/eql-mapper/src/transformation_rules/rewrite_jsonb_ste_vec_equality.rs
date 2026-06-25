use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{
    BinaryOperator, CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr,
    FunctionArgumentList, FunctionArguments, Ident, ObjectName, ObjectNamePart, ValueWithSpan,
};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::ste_vec_ordering::{
    is_eql_typed, is_equality_operator, is_ste_vec_accessor, rhs_as_jsonb,
};
use crate::unifier::Type;
use crate::EqlMapperError;

use super::TransformationRule;

/// Rewrites equality comparisons (`=`, `<>`) on a jsonb STE-vec element
/// extracted via `->` / `->>` / `jsonb_path_query_first` so that the comparison
/// binds to the XOR-aware `eql_v2.eq_term` extractor in EQL 2.3.1 instead of the
/// root `eql_v2_encrypted` equality path.
///
/// # Why this is needed
///
/// In EQL 2.3.1 the `->` operator on `eql_v2_encrypted` returns an
/// `eql_v2.ste_vec_entry` (a single sv element carrying exactly one
/// deterministic equality term — `hm` for bool/null/array/object/root leaves or
/// `oc` for string/number leaves). A bare equality comparison such as
/// `(col -> selector) = $param` does not resolve to the `ste_vec_entry`
/// equality operators because the right-hand side is a root sv query payload
/// (`{"k":"sv", ..., "hm"|"oc": ...}`) which is *not* a full `ste_vec_entry`
/// (it lacks the `s` / `c` fields the `ste_vec_entry` DOMAIN CHECK requires).
/// Postgres instead resolves the comparison to the root `eql_v2_encrypted`
/// equality operators, comparing the wrong (root-scope) terms.
///
/// EQL 2.3.1 provides `eql_v2.eq_term(eql_v2.ste_vec_entry)` which returns
/// `decode(coalesce(entry ->> 'hm', entry ->> 'oc'), 'hex')` — the XOR-aware
/// deterministic equality term. The `ste_vec_entry = ste_vec_entry` operators
/// are defined as `eq_term(a) = eq_term(b)`. This rule rewrites:
///
/// ```sql
/// (col -> selector) = $param
/// ```
///
/// into:
///
/// ```sql
/// eql_v2.eq_term(col -> selector) = decode(coalesce($param ->> 'hm', $param ->> 'oc'), 'hex')
/// ```
///
/// The left operand keeps the accessor (which already yields an
/// `eql_v2.ste_vec_entry` for the `->` form) and wraps it in `eql_v2.eq_term`.
/// The `jsonb_path_query_first` form yields an `eql_v2_encrypted`, so it is
/// first cast to `::JSONB::eql_v2.ste_vec_entry` (the merged element carries the
/// required `s` / `c` fields, so the DOMAIN CHECK passes).
///
/// The right operand is the query payload jsonb. There is no `eq_term(jsonb)`
/// companion overload, so the `eq_term` body is inlined directly on the raw
/// jsonb: `decode(coalesce($param ->> 'hm', $param ->> 'oc'), 'hex')`. The
/// param/literal is reduced to a bare `::JSONB` value (the outer
/// `::eql_v2_encrypted` cast applied by [`super::CastParamsAsEncrypted`] /
/// [`super::CastLiteralsAsEncrypted`] is stripped) because `->>` reads the
/// `hm` / `oc` field off the jsonb directly.
///
/// # Operand orientation
///
/// Only the left-operand form (`col -> selector <op> value`) is rewritten — the
/// rule gates on `is_ste_vec_accessor(left)`. The commutative form with the
/// accessor on the *right* (`value = col -> selector`) is intentionally left
/// untouched; it falls back to root `eql_v2_encrypted` equality. This matches
/// the sibling [`super::RewriteJsonbSteVecOrdering`] rule and, critically, the
/// reclassification pass in [`crate::eql_mapper`] gates on the same predicate,
/// so the two passes always agree (a value is never reclassified as a STE-vec
/// term without the SQL also being rewritten). Supporting the commutative form
/// would require normalising both passes in lockstep.
#[derive(Debug)]
pub struct RewriteJsonbSteVecEquality<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteJsonbSteVecEquality<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    /// Returns `true` if `expr` is the `->` / `->>` accessor form, whose result
    /// is already an `eql_v2.ste_vec_entry`.
    fn is_arrow_accessor(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::BinaryOp {
                op: BinaryOperator::Arrow | BinaryOperator::LongArrow,
                ..
            }
        )
    }

    /// Wraps the left-hand sv-element accessor in `eql_v2.eq_term(...)`.
    ///
    /// The `->` / `->>` accessor already yields an `eql_v2.ste_vec_entry`, so it
    /// is passed straight to `eq_term`. The `jsonb_path_query_first` form yields
    /// an `eql_v2_encrypted`, so it is cast to `::JSONB::eql_v2.ste_vec_entry`
    /// first.
    fn wrap_eq_term_lhs(expr: Expr) -> Expr {
        let entry = if Self::is_arrow_accessor(&expr) {
            expr
        } else {
            Self::cast_ste_vec_entry(expr)
        };
        Self::call("eql_v2", "eq_term", vec![entry])
    }

    /// Casts `expr` to `::JSONB::eql_v2.ste_vec_entry`.
    fn cast_ste_vec_entry(expr: Expr) -> Expr {
        let jsonb = Expr::Cast {
            kind: CastKind::DoubleColon,
            expr: Box::new(expr),
            data_type: DataType::JSONB,
            format: None,
        };

        Expr::Cast {
            kind: CastKind::DoubleColon,
            expr: Box::new(jsonb),
            data_type: DataType::Custom(
                ObjectName(vec![
                    ObjectNamePart::Identifier(Ident::new("eql_v2")),
                    ObjectNamePart::Identifier(Ident::new("ste_vec_entry")),
                ]),
                vec![],
            ),
            format: None,
        }
    }

    /// Builds the right-hand-side equality term:
    /// `decode(coalesce(<rhs> ->> 'hm', <rhs> ->> 'oc'), 'hex')`.
    ///
    /// This inlines the body of `eql_v2.eq_term(eql_v2.ste_vec_entry)` against
    /// the raw query-payload jsonb (which carries exactly one of `hm` / `oc` at
    /// its top level), matching whichever deterministic term the column's leaf
    /// carries.
    fn build_eq_term_rhs(expr: Expr) -> Expr {
        let rhs = rhs_as_jsonb(expr);

        let hm = Self::field_text(rhs.clone(), "hm");
        let oc = Self::field_text(rhs, "oc");
        let coalesce = Self::call("", "coalesce", vec![hm, oc]);

        Self::call("", "decode", vec![coalesce, Self::string_literal("hex")])
    }

    /// Builds `<expr> ->> '<field>'`.
    fn field_text(expr: Expr, field: &str) -> Expr {
        Expr::BinaryOp {
            left: Box::new(expr),
            op: BinaryOperator::LongArrow,
            right: Box::new(Self::string_literal(field)),
        }
    }

    /// Builds a single-quoted string literal expression.
    fn string_literal(value: &str) -> Expr {
        Expr::Value(ValueWithSpan {
            value: SqltkValue::SingleQuotedString(value.to_string()),
            span: Span::empty(),
        })
    }

    /// Builds a function call `[schema.]name(args...)`. An empty `schema`
    /// produces an unqualified call (e.g. `coalesce`, `decode`).
    fn call(schema: &str, name: &str, args: Vec<Expr>) -> Expr {
        let mut parts = Vec::new();
        if !schema.is_empty() {
            parts.push(ObjectNamePart::Identifier(Ident::new(schema)));
        }
        parts.push(ObjectNamePart::Identifier(Ident::new(name)));

        Expr::Function(Function {
            name: ObjectName(parts),
            uses_odbc_syntax: false,
            args: FunctionArguments::List(FunctionArgumentList {
                args: args
                    .into_iter()
                    .map(|arg| FunctionArg::Unnamed(FunctionArgExpr::Expr(arg)))
                    .collect(),
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

impl<'ast> TransformationRule<'ast> for RewriteJsonbSteVecEquality<'ast> {
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

                **left = Self::wrap_eq_term_lhs(left_expr);
                **right = Self::build_eq_term_rhs(right_expr);
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((Expr::BinaryOp { left, op, right },)) = node_path.last_1_as::<Expr>() {
            if is_equality_operator(op)
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
