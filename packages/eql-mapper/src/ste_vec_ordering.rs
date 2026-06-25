//! Shared predicates for recognising jsonb STE-vec ordering comparisons.
//!
//! Two independent passes must agree on exactly which comparisons are jsonb
//! STE-vec ordering comparisons:
//!
//! - The SQL rewrite
//!   ([`crate::transformation_rules::RewriteJsonbSteVecOrdering`]) wraps the
//!   operands in `eql_v2.ore_cllw(...)`.
//! - The parameter/literal reclassification (in [`crate::eql_mapper`]) marks the
//!   right-hand-side value as an [`crate::EqlTerm::SteVecTerm`] so it is
//!   encrypted as a CLLW ORE query term (`oc`).
//!
//! If these two passes disagree, the proxy can emit `eql_v2.ore_cllw(...)` SQL
//! against a value that was *not* encrypted as a CLLW ORE term (or vice versa),
//! silently producing wrong results. Keeping the structural predicates in one
//! place ensures they cannot drift apart.

use sqltk::parser::ast::{BinaryOperator, Expr, ObjectName, ObjectNamePart};

/// Returns `true` if `op` is an *ordering* comparison (`<`, `<=`, `>`, `>=`).
///
/// Equality (`=`, `<>`) is deliberately excluded: sv equality is hmac-based and
/// resolves through a different operator path.
pub(crate) fn is_ordering_operator(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Lt | BinaryOperator::LtEq | BinaryOperator::Gt | BinaryOperator::GtEq
    )
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
