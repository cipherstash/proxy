//! Renumbering of the rewritten statement's placeholders.
//!
//! A rewrite rule may drop a placeholder (encrypted JSON equality folds the
//! path operand into the value's needle) or duplicate one. Either way the
//! surviving placeholders no longer read `$1..$n` in order, and PostgreSQL
//! requires a statement's params to be exactly `$1..$m` — an unreferenced `$n`
//! is only accepted when its type is declared, and a gap is a statement whose
//! param count no longer matches what the client bound.
//!
//! So after the rewrite rules have run, this pass walks the new statement in
//! SQL order and renumbers the placeholders `$1..$m`, recording which input
//! param each output position came from. That record is the raw material for a
//! [`crate::ParamPlan`].
//!
//! This runs as a **separate pass, after** the rewrite rules, which is what
//! lets [`crate::FailOnPlaceholderChange`] keep enforcing that no rule replaces
//! a placeholder with a literal during the rewrite itself.

use std::collections::HashSet;

use sqltk::parser::ast::{self, DataType, Expr, ObjectNamePart};
use sqltk::{NodePath, Transform, Visitable};

use crate::{EqlMapperError, Param};

/// The schema the query-operand twin domains live in.
const EQL_V3_SCHEMA: &str = "eql_v3";

/// The prefix of every query-operand twin domain, e.g. `query_text_search`.
const QUERY_TWIN_PREFIX: &str = "query_";

/// Assigns `$1..$m` to the placeholders of a rewritten statement in SQL order.
#[derive(Debug, Default)]
pub(crate) struct RenumberParams {
    /// The input param each output position was renumbered from, in output
    /// order. Index `i` holds the source of `$(i + 1)`.
    sources: Vec<Param>,

    /// The output params that carry a *query* operand, as opposed to a value
    /// being stored.
    ///
    /// This has to be decided per output occurrence rather than per input param.
    /// One placeholder can be bound in both roles at once —
    /// `UPDATE t SET enc = $1 WHERE enc = $1` stores `$1` and queries with it —
    /// and marking the whole input param as a query operand strips the
    /// ciphertext from the stored value, so its cast to the column's own domain
    /// fails the domain CHECK.
    ///
    /// The rewritten statement is the authority: the rewrite has already cast
    /// each operand to the domain its position requires, so a cast to an
    /// `eql_v3.query_*` twin *is* the statement saying "this one is a query
    /// operand".
    query_operands: HashSet<Param>,
}

impl RenumberParams {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn into_parts(self) -> (Vec<Param>, HashSet<Param>) {
        (self.sources, self.query_operands)
    }
}

/// Whether `data_type` names one of the `eql_v3.query_*` twin domains.
fn is_query_twin(data_type: &DataType) -> bool {
    let DataType::Custom(name, _) = data_type else {
        return false;
    };

    let [ObjectNamePart::Identifier(schema), ObjectNamePart::Identifier(domain)] = &name.0[..]
    else {
        return false;
    };

    schema.value == EQL_V3_SCHEMA && domain.value.starts_with(QUERY_TWIN_PREFIX)
}

/// The placeholder a cast wraps, if it wraps exactly one.
///
/// `cast_expr_to_v3_domain` builds `<expr>::JSONB::<schema>.<domain>`, so the
/// placeholder sits two casts down. It has already been renumbered by the time
/// the enclosing cast is visited — transformation is depth-first — so the name
/// read here is the *output* param.
fn wrapped_placeholder(expr: &Expr) -> Option<Param> {
    let Expr::Cast { expr, .. } = expr else {
        return None;
    };

    let Expr::Value(value) = expr.as_ref() else {
        return None;
    };

    let ast::Value::Placeholder(name) = &value.value else {
        return None;
    };

    Param::try_from(name).ok()
}

impl<'ast> Transform<'ast> for RenumberParams {
    type Error = EqlMapperError;

    fn transform<N: Visitable>(
        &mut self,
        _node_path: &NodePath<'ast>,
        mut target_node: N,
    ) -> Result<N, Self::Error> {
        // Transformation is depth-first and visits siblings in order, so
        // placeholders arrive in the order they appear in the rendered SQL.
        if let Some(ast::Value::Placeholder(name)) = target_node.downcast_mut::<ast::Value>() {
            let source = Param::try_from(&*name)?;
            self.sources.push(source);
            *name = format!("${}", self.sources.len());
        }

        // Depth-first, so a cast is visited after the placeholder it wraps: the
        // placeholder already carries its output number by the time we get here.
        if let Some(Expr::Cast {
            expr, data_type, ..
        }) = target_node.downcast_ref::<Expr>()
        {
            if is_query_twin(data_type) {
                if let Some(param) = wrapped_placeholder(expr) {
                    self.query_operands.insert(param);
                }
            }
        }

        Ok(target_node)
    }
}
