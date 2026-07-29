use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::{
    Expr, OrderBy, OrderByKind, Query, Select, SelectItem, SetExpr, Value as SqltkValue,
};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{DomainIdentity, Type, Value};
use crate::EqlMapperError;

use super::helpers::eql_v3_term_call;
use super::TransformationRule;

/// Rewrites `ORDER BY <ordinal>` that refers to an encrypted column so it sorts
/// by the column's **ordering term**:
///
/// ```sql
/// SELECT enc FROM t ORDER BY 1
/// -- becomes
/// SELECT enc FROM t ORDER BY eql_v3.ord_term(enc)
/// ```
///
/// [`super::RewriteEqlOrderBy`] does this for a named column, but it matches on
/// the `ORDER BY` expression's own type and an ordinal is just a number — its
/// type says nothing about the column it selects. Left alone, the sort falls
/// back to jsonb ordering over the randomised ciphertext, which is not merely
/// wrong but differently wrong on every insert.
///
/// Resolving the ordinal needs the projection, which is a *sibling* of the
/// `ORDER BY` (both hang off the [`Query`]), not an ancestor — so this cannot
/// be done from the `OrderByExpr` the other rule operates on. Transformation is
/// depth-first, so by the time the `Query` is reached that rule has already
/// looked at the ordinal and skipped it; this rule owns the ordinal case
/// outright rather than trying to feed the other one.
///
/// Replacing the ordinal with the projected expression is
/// semantics-preserving: PostgreSQL defines `ORDER BY n` as ordering by the
/// n-th output column.
#[derive(Debug)]
pub struct RewriteEqlOrdinalOrderBy<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlOrdinalOrderBy<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    fn eql_identity_of(&self, expr: &'ast Expr) -> Option<DomainIdentity> {
        match self.node_types.get(&NodeKey::new(expr)) {
            Some(Type::Value(Value::Eql(eql_term))) => {
                Some(eql_term.eql_value().domain_identity().clone())
            }
            _ => None,
        }
    }

    /// The `SELECT` a query projects from, if it has one.
    pub(crate) fn select_of(query: &Query) -> Option<&Select> {
        match query.body.as_ref() {
            SetExpr::Select(select) => Some(select),
            _ => None,
        }
    }

    /// The 1-based ordinal an `ORDER BY`/`GROUP BY` expression is, if it is one.
    pub(crate) fn ordinal_of(expr: &Expr) -> Option<usize> {
        let Expr::Value(value) = expr else {
            return None;
        };

        let SqltkValue::Number(n, _) = &value.value else {
            return None;
        };

        n.to_string().parse::<usize>().ok().filter(|n| *n > 0)
    }

    /// The expression the n-th projected column selects, if it is a plain one.
    pub(crate) fn projected_expr(select: &Select, ordinal: usize) -> Option<&Expr> {
        match select.projection.get(ordinal - 1)? {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => Some(expr),
            // A wildcard hides which column the ordinal selects.
            _ => None,
        }
    }

    /// The encrypted column an ordinal selects, along with the expression that
    /// names it, or `None` if it selects a plaintext column or cannot be
    /// resolved.
    fn ordinal_target(
        &self,
        select: &'ast Select,
        expr: &Expr,
    ) -> Option<(DomainIdentity, &'ast Expr)> {
        let ordinal = Self::ordinal_of(expr)?;
        let projected = Self::projected_expr(select, ordinal)?;

        self.eql_identity_of(projected)
            .map(|identity| (identity, projected))
    }
}

impl<'ast> TransformationRule<'ast> for RewriteEqlOrdinalOrderBy<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Read the shape from the ORIGINAL query — `node_types` is keyed by it.
        let Some((original,)) = node_path.last_1_as::<Query>() else {
            return Ok(false);
        };

        let Some(original_select) = Self::select_of(original) else {
            return Ok(false);
        };

        let Some(OrderBy {
            kind: OrderByKind::Expressions(original_order_by),
            ..
        }) = &original.order_by
        else {
            return Ok(false);
        };

        // Resolve every ordinal against the original projection first, so the
        // target is only touched if there is something to rewrite.
        let targets = original_order_by
            .iter()
            .map(|obe| self.ordinal_target(original_select, &obe.expr))
            .collect::<Vec<_>>();

        if targets.iter().all(Option::is_none) {
            return Ok(false);
        }

        let Some(target) = target_node.downcast_mut::<Query>() else {
            return Ok(false);
        };

        let Some(OrderBy {
            kind: OrderByKind::Expressions(order_by),
            ..
        }) = &mut target.order_by
        else {
            return Ok(false);
        };

        for (obe, target) in order_by.iter_mut().zip(targets.iter()) {
            let Some((identity, projected)) = target else {
                continue;
            };

            let Some(term_fn) = identity.ord_term_fn() else {
                return Err(EqlMapperError::Transform(format!(
                    "encrypted column {} cannot be used in ORDER BY (domain {} carries no ordering term)",
                    identity.token, identity.domain.value
                )));
            };

            // The projected expression is cloned from the original rather than
            // taken from the rewritten projection: the projection may have been
            // wrapped (by `grouped_value`, say), and what is wanted here is the
            // column itself.
            let _ = mem::replace(&mut obe.expr, Expr::Value(SqltkValue::Null.into()));
            obe.expr = eql_v3_term_call(term_fn, (*projected).clone());
        }

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        let Some((original,)) = node_path.last_1_as::<Query>() else {
            return false;
        };

        let (
            Some(select),
            Some(OrderBy {
                kind: OrderByKind::Expressions(order_by),
                ..
            }),
        ) = (Self::select_of(original), &original.order_by)
        else {
            return false;
        };

        order_by
            .iter()
            .any(|obe| self.ordinal_target(select, &obe.expr).is_some())
    }
}
