use std::collections::HashMap;
use std::mem;

use sqltk::parser::{
    ast::{
        BinaryOperator, CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr,
        FunctionArgumentList, FunctionArguments, Ident, ObjectName, ObjectNamePart,
        Value as SqltkValue, ValueWithSpan,
    },
    tokenizer::Span,
};
use sqltk::NodeKey;

use crate::unifier::{DomainIdentity, EqlTerm, Type, Value};

/// The term function for comparison operator `op` on a column with `identity`,
/// or `None` if the domain provides no term for that operator.
///
/// Shared by [`super::RewriteEqlComparisonOps`] and
/// [`super::RewriteEqlAnyAllOps`], which rewrite the same comparison in its
/// scalar and array-quantified spellings.
pub(crate) fn term_fn_for(op: &BinaryOperator, identity: &DomainIdentity) -> Option<&'static str> {
    match op {
        BinaryOperator::Eq | BinaryOperator::NotEq => identity.eq_term_fn(),
        BinaryOperator::Lt | BinaryOperator::LtEq | BinaryOperator::Gt | BinaryOperator::GtEq => {
            identity.ord_term_fn()
        }
        _ => None,
    }
}

/// The v3 domain an encrypted **query operand** — the value side of a
/// predicate — casts to, or `None` if it takes no cast.
///
/// - [`EqlTerm::JsonAccessor`] / [`EqlTerm::JsonPath`] — no cast. A JSON field
///   selector is passed to the eql_v3 function as bare encrypted *text*
///   (`eql_v3."->"(json, text)`), not as a jsonb query payload.
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
/// - Everything else — the column domain's `eql_v3.query_*` twin, which carries
///   only the search terms the predicate needs, not a whole ciphertext.
pub(crate) fn query_operand_domain(eql_term: &EqlTerm) -> Option<(String, String)> {
    match eql_term {
        EqlTerm::JsonAccessor(_) | EqlTerm::JsonPath(_) => None,
        EqlTerm::JsonOrd(_) => Some(("eql_v3".to_string(), "query_integer_ord".to_string())),
        EqlTerm::JsonValueSelector(_) => Some(("eql_v3".to_string(), "query_json".to_string())),
        _ => {
            let (schema, twin) = eql_term.eql_value().domain_identity().query_twin();
            Some((schema.to_string(), twin))
        }
    }
}

/// The v3 domain an encrypted **full-payload** operand casts to: the column's
/// own domain, carrying the ciphertext plus every search term the column
/// indexes.
///
/// This is what an `INSERT` value, an `UPDATE` assignment and a containment
/// needle all need — as opposed to a predicate operand, which needs only the
/// terms of [`query_operand_domain`].
///
/// Returns `None` for a JSON selector, which is bare text in every position.
pub(crate) fn full_payload_domain(eql_term: &EqlTerm) -> Option<(String, String)> {
    match eql_term {
        EqlTerm::JsonAccessor(_) | EqlTerm::JsonPath(_) => None,
        _ => Some((
            "public".to_string(),
            eql_term.eql_value().domain_identity().domain.value.clone(),
        )),
    }
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

/// Casts `target` — the already-transformed form of `original` — to the v3
/// domain `domain_of` chooses for its EQL type.
///
/// Only a literal or placeholder is cast. A column reference is already of its
/// domain type, and any other expression belongs to whichever rule owns it.
/// Returns `true` if a cast was applied.
///
/// This is called by the rule that *owns the context* — a comparison, a match,
/// a containment, an INSERT value — so the choice of domain never has to be
/// inferred from where the node happens to sit in the tree.
pub(crate) fn cast_encrypted_operand(
    node_types: &HashMap<NodeKey<'_>, Type>,
    original: &Expr,
    target: &mut Expr,
    domain_of: fn(&EqlTerm) -> Option<(String, String)>,
) -> bool {
    if !matches!(original, Expr::Value(_)) {
        return false;
    }

    let Some(Type::Value(Value::Eql(eql_term))) = node_types.get(&NodeKey::new(original)) else {
        return false;
    };

    let Some((schema, domain)) = domain_of(eql_term) else {
        return false;
    };

    let wrapped = mem::replace(
        target,
        Expr::Value(ValueWithSpan {
            value: SqltkValue::Null,
            span: Span::empty(),
        }),
    );

    *target = cast_expr_to_v3_domain(wrapped, &schema, &domain);
    true
}

/// Builds `<wrapped>::JSONB::<schema>.<domain>` around an arbitrary expression.
pub(crate) fn cast_expr_to_v3_domain(wrapped: Expr, schema: &str, domain: &str) -> Expr {
    let cast_jsonb = Expr::Cast {
        kind: CastKind::DoubleColon,
        expr: Box::new(wrapped),
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
