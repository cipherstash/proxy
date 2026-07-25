use sqltk::parser::{
    ast::{
        BinaryOperator, CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr,
        FunctionArgumentList, FunctionArguments, Ident, ObjectName, ObjectNamePart,
    },
    tokenizer::Span,
};
use sqltk::NodePath;

use crate::unifier::{DomainIdentity, EqlTerm};

/// The v3 cast target `(schema, domain)` for a JSON ordering operand
/// ([`EqlTerm::JsonOrd`]) — always the shape-only scalar ord twin
/// `eql_v3.query_integer_ord`, regardless of the JSON leaf's scalar type.
///
/// `eql_v3.ord_term` is type-agnostic (it extracts the `op` bytes as
/// `ope_cllw` and compares them bytewise), and `query_integer_ord`'s domain
/// CHECK is shape-only (`{v,i,op}`, no `c`). So a single twin serves numbers,
/// text, dates, etc. — which is what makes JSON range work in the extended
/// protocol, where the operand's scalar type is unknown at rewrite time.
/// Returns `None` for any other term.
/// The v3 cast target `(schema, domain)` for an encrypted-JSON *query operand*
/// whose domain is fixed by the operand's role rather than by the column's
/// domain identity. Returns `None` for any other term.
///
/// - [`EqlTerm::JsonOrd`] — the shape-only scalar ord twin
///   `eql_v3.query_integer_ord`, regardless of the JSON leaf's scalar type.
///   `eql_v3.ord_term` is type-agnostic (it extracts the `op` bytes as
///   `ope_cllw` and compares them bytewise) and the twin's domain CHECK is
///   shape-only (`{v,i,op}`, no `c`), so one twin serves numbers, text, dates.
///   That is what makes JSON range work in the extended protocol, where the
///   operand's scalar type is unknown at rewrite time.
/// - [`EqlTerm::JsonValueSelector`] — `eql_v3.query_json`, the containment
///   needle domain. The fused value selector is a one-entry, term-less
///   containment payload (`{sv: [{s}]}`), which is what
///   `eql_v3.jsonb_contains` matches against.
pub(crate) fn json_query_operand_cast_target(eql_term: &EqlTerm) -> Option<(String, String)> {
    let domain = match eql_term {
        EqlTerm::JsonOrd(_) => "query_integer_ord",
        EqlTerm::JsonValueSelector(_) => "query_json",
        _ => return None,
    };

    Some(("eql_v3".to_string(), domain.to_string()))
}

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
