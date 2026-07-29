use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{Expr, GroupByExpr, Select, SelectItem, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{EqlValue, Type, Value};
use crate::EqlMapperError;

use super::helpers::eql_v3_term_call;
use super::preserve_effective_aliases::derive_effective_alias;
use super::RewriteEqlOrdinalOrderBy;
use super::TransformationRule;

/// Rewrites `GROUP BY` on an encrypted column to group by its **equality term**,
/// and lifts any projection of that column through an aggregate so the query
/// stays valid:
///
/// ```sql
/// SELECT col, COUNT(*) FROM t GROUP BY col
/// -- becomes
/// SELECT eql_v3.grouped_value(col) AS col, COUNT(*) FROM t GROUP BY eql_v3.eq_term(col)
/// ```
///
/// **Without this rewrite the grouping is silently wrong.** An encrypted column
/// is a domain over `jsonb`, so a bare `GROUP BY` groups on the whole payload —
/// including `c`, the ciphertext, which is randomised per encryption. Two rows
/// holding the *same* plaintext land in different groups, so `GROUP BY` degrades
/// into `GROUP BY <every row>`.
///
/// Grouping is equality, so the key is the same `eq_term` an `=` comparison uses
/// (`ord_term` for a domain that stores no `hm`).
///
/// Once the key is `eq_term(col)`, PostgreSQL no longer sees the bare column as
/// functionally dependent on it and would reject `SELECT col`.
/// `eql_v3.grouped_value` — the aggregate EQL provides for exactly this — returns
/// one representative value per group, which is enough because every row in a
/// group is an encryption of the same plaintext. The original projection name is
/// preserved, so clients selecting by name are unaffected.
///
/// Requires an EQL release carrying `eql_v3.grouped_value` (CIP-3657, EQL PR
/// 423), which the pinned 3.0.4 does. Only the projection case needs it;
/// grouping without selecting the column does not.
#[derive(Debug)]
pub struct RewriteEqlGroupBy<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlGroupBy<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    fn eql_value_of(&self, expr: &'ast Expr) -> Option<EqlValue> {
        match self.node_types.get(&NodeKey::new(expr)) {
            Some(Type::Value(Value::Eql(eql_term))) => Some(eql_term.eql_value().clone()),
            _ => None,
        }
    }

    /// The encrypted columns a `GROUP BY` groups on, in order.
    ///
    /// A key may be written as an ordinal (`GROUP BY 1`), which names no column
    /// of its own, so it is resolved against the projection — otherwise the key
    /// is left ungrouped and every row becomes its own group. The projected
    /// expression is carried along so the ordinal can be replaced by the term
    /// applied to the column it selects.
    fn grouped_eql_values(
        &self,
        select: &'ast Select,
    ) -> Vec<Option<(EqlValue, Option<&'ast Expr>)>> {
        match &select.group_by {
            GroupByExpr::Expressions(exprs, _) => exprs
                .iter()
                .map(|expr| match self.eql_value_of(expr) {
                    Some(eql_value) => Some((eql_value, None)),
                    None => {
                        let ordinal = RewriteEqlOrdinalOrderBy::ordinal_of(expr)?;
                        let projected = RewriteEqlOrdinalOrderBy::projected_expr(select, ordinal)?;

                        self.eql_value_of(projected)
                            .map(|eql_value| (eql_value, Some(projected)))
                    }
                })
                .collect(),
            GroupByExpr::All(_) => vec![],
        }
    }

    /// The expression a select item projects, if it is a plain one.
    fn select_item_expr(item: &'ast SelectItem) -> Option<&'ast Expr> {
        match item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => Some(expr),
            _ => None,
        }
    }

    /// Whether `item` projects one of the grouped encrypted columns *directly*,
    /// and so must be lifted through an aggregate.
    ///
    /// Matched on the resolved column rather than on syntax, so `SELECT t.col …
    /// GROUP BY col` — which PostgreSQL accepts today — keeps working.
    ///
    /// "Directly" excludes an expression that merely *contains* the column.
    /// `MIN(col)` resolves to the same [`EqlValue`] as `col` itself, so without
    /// this it would be lifted too, producing `grouped_value(eql_v3.min(col))` —
    /// an aggregate inside an aggregate, which PostgreSQL rejects. An aggregate
    /// already yields one value per group and needs no lifting.
    fn projects_grouped_column(&self, item: &'ast SelectItem, grouped: &[EqlValue]) -> bool {
        let Some(expr) = Self::select_item_expr(item) else {
            return false;
        };

        if !Self::is_direct_column_reference(expr) {
            return false;
        }

        self.eql_value_of(expr)
            .is_some_and(|value| grouped.contains(&value))
    }

    /// Whether `expr` names a column and nothing more.
    fn is_direct_column_reference(expr: &'ast Expr) -> bool {
        match expr {
            Expr::Identifier(_) | Expr::CompoundIdentifier(_) => true,
            Expr::Nested(inner) => Self::is_direct_column_reference(inner),
            _ => false,
        }
    }
}

impl<'ast> TransformationRule<'ast> for RewriteEqlGroupBy<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Read the encrypted columns from the ORIGINAL select — `node_types` is
        // keyed by it, and the target's children are already rewritten.
        let Some((original,)) = node_path.last_1_as::<Select>() else {
            return Ok(false);
        };

        let grouped = self.grouped_eql_values(original);
        if grouped.iter().all(Option::is_none) {
            return Ok(false);
        }

        // A wildcard hides the projected columns, so a grouped encrypted column
        // cannot be lifted through `grouped_value` — and left alone it is no
        // longer functionally dependent on the rewritten key, which PostgreSQL
        // rejects with "column must appear in the GROUP BY clause". Rejecting
        // here names the query shape instead.
        //
        // Consistent with `SELECT DISTINCT *`, which is refused for the same
        // reason: the projection has to be written out for the rewrite to reach
        // the encrypted columns in it.
        if original
            .projection
            .iter()
            .any(|item| Self::select_item_expr(item).is_none())
        {
            return Err(EqlMapperError::Transform(
                "SELECT * cannot be combined with GROUP BY on an encrypted column: grouping uses \
                 the column's equality term, so the column has to be listed explicitly to be \
                 projected through eql_v3.grouped_value"
                    .to_string(),
            ));
        }

        let Some(target) = target_node.downcast_mut::<Select>() else {
            return Ok(false);
        };

        // Group by the equality term.
        if let GroupByExpr::Expressions(exprs, _) = &mut target.group_by {
            for (expr, grouped) in exprs.iter_mut().zip(grouped.iter()) {
                let Some((eql_value, projected)) = grouped else {
                    continue;
                };

                let identity = eql_value.domain_identity();
                let Some(term_fn) = identity.eq_term_fn() else {
                    return Err(EqlMapperError::Transform(format!(
                        "encrypted column {} cannot be used in GROUP BY (domain {} carries no equality term)",
                        identity.token, identity.domain.value
                    )));
                };

                // An ordinal names nothing to wrap, so the column it selects
                // is substituted for it; a named key wraps in place.
                let grouped_expr = match projected {
                    Some(projected) => (*projected).clone(),
                    None => mem::replace(
                        expr,
                        Expr::Value(ValueWithSpan {
                            value: SqltkValue::Null,
                            span: Span::empty(),
                        }),
                    ),
                };
                *expr = eql_v3_term_call(term_fn, grouped_expr);
            }
        }

        // Lift any projection of a grouped column through `any_value`, keeping
        // the name the client asked for.
        let grouped: Vec<EqlValue> = grouped
            .into_iter()
            .flatten()
            .map(|(eql_value, _)| eql_value)
            .collect();
        for (original_item, target_item) in
            original.projection.iter().zip(target.projection.iter_mut())
        {
            if !self.projects_grouped_column(original_item, &grouped) {
                continue;
            }

            let alias = derive_effective_alias(original_item);
            let (SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. }) =
                target_item
            else {
                continue;
            };

            let projected = mem::replace(
                expr,
                Expr::Value(ValueWithSpan {
                    value: SqltkValue::Null,
                    span: Span::empty(),
                }),
            );
            let aggregated = eql_v3_term_call("grouped_value", projected);

            *target_item = match alias {
                Some(alias) => SelectItem::ExprWithAlias {
                    expr: aggregated,
                    alias,
                },
                None => SelectItem::UnnamedExpr(aggregated),
            };
        }

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        match node_path.last_1_as::<Select>() {
            Some((original,)) => self
                .grouped_eql_values(original)
                .iter()
                .any(Option::is_some),
            None => false,
        }
    }
}
