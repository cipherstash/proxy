use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{
    Expr, OrderBy, OrderByKind, Query, Select, SelectItem, SetExpr, Value as SqltkValue,
};

use crate::{
    inference::{InferType, TypeError},
    EqlTrait, TypeInferencer,
};

/// The expression an `ORDER BY` or `GROUP BY` key ultimately acts on.
///
/// A key may be written as an ordinal, which selects the n-th projected column
/// and names nothing itself, so a bound belongs on that column rather than on
/// the number. Anything that cannot be resolved — an ordinal past the end of the
/// projection, or one selecting a wildcard — is returned unchanged, to be typed
/// as written.
pub(crate) fn resolve_positional_key<'ast>(
    select: Option<&'ast Select>,
    expr: &'ast Expr,
) -> &'ast Expr {
    let Some(select) = select else { return expr };

    let Expr::Value(value) = expr else {
        return expr;
    };

    let SqltkValue::Number(n, _) = &value.value else {
        return expr;
    };

    let Ok(ordinal) = n.to_string().parse::<usize>() else {
        return expr;
    };

    match select.projection.get(ordinal.wrapping_sub(1)) {
        Some(SelectItem::UnnamedExpr(projected))
        | Some(SelectItem::ExprWithAlias {
            expr: projected, ..
        }) => projected,
        _ => expr,
    }
}

#[trace_infer]
impl<'ast> InferType<'ast, Query> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, query: &'ast Query) -> Result<(), TypeError> {
        let Query { body, order_by, .. } = query;

        self.unify_nodes(query, &**body)?;

        // Sorting compares values, so every `ORDER BY` key needs an ordering
        // term. Without this the clause is never given a type at all, and
        // `ORDER BY 1` over an encrypted column sorts on the raw jsonb payload —
        // whose ciphertext is randomised, so the order differs on every insert.
        if let Some(OrderBy { kind, .. }) = order_by {
            match kind {
                OrderByKind::Expressions(exprs) => {
                    let select = match body.as_ref() {
                        SetExpr::Select(select) => Some(&**select),
                        _ => None,
                    };

                    for order_by_expr in exprs {
                        let key = resolve_positional_key(select, &order_by_expr.expr);
                        self.unify_node_with_bound(key, EqlTrait::Ord)?;
                    }
                }

                // `ORDER BY ALL` (DuckDB/ClickHouse syntax) names every
                // projected column without listing any, so there is no
                // expression to bound. PostgreSQL rejects the syntax anyway;
                // rejecting it here keeps the clause from ever passing through
                // unconstrained.
                OrderByKind::All(_) => {
                    return Err(TypeError::UnsupportedSqlFeature("ORDER BY ALL".into()));
                }
            }
        }

        Ok(())
    }
}
