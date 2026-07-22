use sqltk::parser::{
    ast::{
        BinaryOperator, CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr,
        FunctionArgumentList, FunctionArguments, Ident, ObjectName, ObjectNamePart,
    },
    tokenizer::Span,
};
use sqltk::NodePath;

use crate::unifier::DomainIdentity;

/// The scalar comparison operators the v3 term-function rewrite handles.
pub(crate) fn is_comparison_op(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq
    )
}

/// Whether an encrypted value at `node_path` is a **query operand** (the RHS of a
/// comparison or match predicate) rather than a **stored value** (an INSERT
/// `VALUES` item or UPDATE `SET` target). Walks the enclosing `Expr` ancestor
/// chain looking for a comparison `BinaryOp` or a `LIKE`/`ILIKE` predicate. The
/// traversal is post-order, so when a cast rule runs on the operand the enclosing
/// predicate is still intact in the path.
fn is_query_operand(node_path: &NodePath<'_>) -> bool {
    let mut depth = 1;
    while let Some(expr) = node_path.nth_last_as::<Expr>(depth) {
        match expr {
            Expr::BinaryOp { op, .. }
                if is_comparison_op(op) || matches!(op, BinaryOperator::AtAt) =>
            {
                return true
            }
            Expr::Like { .. } | Expr::ILike { .. } => return true,
            _ => {}
        }
        depth += 1;
    }
    false
}

/// The v3 cast target `(schema, domain typname)` for an encrypted value carrying
/// `identity` at `node_path`. A query operand casts to the `eql_v3.query_*` twin
/// (term-only payload); a stored value casts to the `public` column domain.
pub(crate) fn v3_cast_target(
    node_path: &NodePath<'_>,
    identity: &DomainIdentity,
) -> (String, String) {
    if is_query_operand(node_path) {
        let (schema, twin) = identity.query_twin();
        (schema.to_string(), twin)
    } else {
        ("public".to_string(), identity.domain.value.clone())
    }
}

/// Builds `<wrapped>::JSONB::<schema>.<domain>` — the cast that wraps an encrypted
/// value (a jsonb payload) as an EQL v3 domain. `schema` is `public` for a stored
/// column domain and `eql_v3` for a query-operand twin.
pub(crate) fn cast_to_v3_domain(
    wrapped: sqltk::parser::ast::Value,
    schema: &str,
    domain: &str,
) -> Expr {
    let cast_jsonb = Expr::Cast {
        kind: CastKind::DoubleColon,
        expr: Box::new(Expr::Value(sqltk::parser::ast::ValueWithSpan {
            value: wrapped,
            span: Span::empty(),
        })),
        data_type: DataType::JSONB,
        format: None,
    };

    let domain_type = ObjectName(vec![
        ObjectNamePart::Identifier(Ident::new(schema)),
        ObjectNamePart::Identifier(Ident::new(domain)),
    ]);

    Expr::Cast {
        kind: CastKind::DoubleColon,
        expr: Box::new(cast_jsonb),
        data_type: DataType::Custom(domain_type, vec![]),
        format: None,
    }
}

/// Builds `eql_v3.<fn_name>(<arg>)` — a call to an EQL v3 term-extraction function
/// (`eq_term`, `ord_term`, `ord_term_ore`, `match_term`).
pub(crate) fn eql_v3_term_call(fn_name: &str, arg: Expr) -> Expr {
    Expr::Function(Function {
        name: ObjectName(vec![
            ObjectNamePart::Identifier(Ident::new("eql_v3")),
            ObjectNamePart::Identifier(Ident::new(fn_name)),
        ]),
        uses_odbc_syntax: false,
        args: FunctionArguments::List(FunctionArgumentList {
            args: vec![FunctionArg::Unnamed(FunctionArgExpr::Expr(arg))],
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
