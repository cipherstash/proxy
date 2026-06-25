//! Shared predicates and operand helpers for recognising and rewriting jsonb
//! STE-vec *term* comparisons (both ordering and equality).
//!
//! Two independent passes must agree on exactly which comparisons are jsonb
//! STE-vec term comparisons:
//!
//! - The SQL rewrites
//!   ([`crate::transformation_rules::RewriteJsonbSteVecOrdering`] wraps the
//!   operands in `eql_v2.ore_cllw(...)`;
//!   [`crate::transformation_rules::RewriteJsonbSteVecEquality`] binds them to
//!   `eql_v2.eq_term(...)`).
//! - The parameter/literal reclassification (in [`crate::eql_mapper`]) marks the
//!   right-hand-side value as an [`crate::EqlTerm::SteVecTerm`] so it is
//!   encrypted as the matching STE-vec query term (`oc` for CLLW ORE leaves,
//!   `hm` for hmac/term-filter leaves).
//!
//! If these two passes disagree, the proxy can emit STE-vec SQL against a value
//! that was *not* encrypted as a STE-vec query term (or vice versa), silently
//! producing wrong results. Keeping the structural predicates **and** the
//! encryption-binding-critical operand helpers ([`is_eql_typed`],
//! [`rhs_as_jsonb`]) in one place ensures they cannot drift apart.

use std::collections::HashMap;

use sqltk::parser::ast::{BinaryOperator, CastKind, DataType, Expr, ObjectName, ObjectNamePart};
use sqltk::NodeKey;

use crate::unifier::{Type, Value};

/// Returns `true` if `op` is an *ordering* comparison (`<`, `<=`, `>`, `>=`).
///
/// Equality (`=`, `<>`) is deliberately excluded: sv equality resolves through
/// the `eql_v2.eq_term` operator path (see [`is_equality_operator`] and
/// [`crate::transformation_rules::RewriteJsonbSteVecEquality`]), not the CLLW
/// ORE ordering path.
pub(crate) fn is_ordering_operator(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Lt | BinaryOperator::LtEq | BinaryOperator::Gt | BinaryOperator::GtEq
    )
}

/// Returns `true` if `op` is an *equality* comparison (`=`, `<>`).
///
/// sv equality binds to the XOR-aware `eql_v2.eq_term` extractor (which
/// coalesces a leaf's `hm`/`oc` term), so it is rewritten separately from the
/// ordering comparisons handled by [`is_ordering_operator`].
pub(crate) fn is_equality_operator(op: &BinaryOperator) -> bool {
    matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq)
}

/// Returns `true` if `expr` extracts a single STE-vec element from an EQL jsonb
/// column — i.e. it is the result of `->` / `->>` or `jsonb_path_query_first`.
///
/// These are the expressions whose value is a single sv element (carrying a
/// CLLW ORE `oc` term) rather than a root `eql_v2_encrypted` value, so an
/// ordering comparison against them must be rewritten to compare CLLW ORE terms.
pub(crate) fn is_ste_vec_accessor(expr: &Expr) -> bool {
    match expr {
        Expr::BinaryOp {
            op: BinaryOperator::Arrow | BinaryOperator::LongArrow,
            ..
        } => true,
        Expr::Function(function) => is_jsonb_path_query_first(&function.name),
        _ => false,
    }
}

/// Matches `jsonb_path_query_first` and its `eql_v2.`-qualified rewritten form
/// (case-insensitively).
fn is_jsonb_path_query_first(name: &ObjectName) -> bool {
    let parts: Vec<String> = name
        .0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) => ident.value.to_lowercase(),
        })
        .collect();

    matches!(parts.as_slice(), [f] if f == "jsonb_path_query_first")
        || matches!(parts.as_slice(), [schema, f] if schema == "eql_v2" && f == "jsonb_path_query_first")
}

/// Returns `true` if `expr` is EQL-typed in `node_types`.
///
/// Shared by the ordering and equality rewrite rules so both bind the rewrite
/// to exactly the same operands. Kept here (rather than duplicated per rule)
/// because mis-identifying an operand as EQL-typed would bind a rewrite to a
/// value encrypted differently than the SQL extracts it.
pub(crate) fn is_eql_typed(node_types: &HashMap<NodeKey<'_>, Type>, expr: &Expr) -> bool {
    matches!(
        node_types.get(&NodeKey::new(expr)),
        Some(Type::Value(Value::Eql(_)))
    )
}

/// Reduces a STE-vec comparison's right-hand-side operand to a bare `::JSONB`
/// value.
///
/// The casting rules wrap encrypted params/literals as
/// `<value>::JSONB::eql_v2_encrypted`. The STE-vec extractors
/// (`eql_v2.ore_cllw(jsonb)` for ordering, the inlined `eq_term` `->>` reads for
/// equality) operate on raw `jsonb`, so the outer `::eql_v2_encrypted` cast is
/// stripped, leaving `<value>::JSONB`. If the expression is not in the expected
/// double-cast shape it is wrapped in a `::JSONB` cast defensively.
///
/// This cast-shape logic is encryption-binding-critical and shared by both
/// rewrite rules: a divergence would bind a value encrypted one way to SQL that
/// extracts it another way, silently producing wrong results.
pub(crate) fn rhs_as_jsonb(expr: Expr) -> Expr {
    if let Expr::Cast {
        kind: CastKind::DoubleColon,
        expr: inner,
        data_type: DataType::Custom(name, _),
        ..
    } = &expr
    {
        let is_encrypted = name.0.len() == 1
            && matches!(
                &name.0[0],
                ObjectNamePart::Identifier(ident)
                    if ident.value.eq_ignore_ascii_case("eql_v2_encrypted")
            );

        if is_encrypted {
            if let Expr::Cast {
                data_type: DataType::JSONB,
                ..
            } = &**inner
            {
                // Strip the outer `::eql_v2_encrypted` cast, keeping `<value>::JSONB`.
                return (**inner).clone();
            }
        }
    }

    // Defensive fallback: cast whatever we have to JSONB.
    Expr::Cast {
        kind: CastKind::DoubleColon,
        expr: Box::new(expr),
        data_type: DataType::JSONB,
        format: None,
    }
}
