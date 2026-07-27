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
/// 423) — later than the 3.0.2 currently pinned in `mise.toml`. Only the
/// projection case needs it; grouping without selecting the column does not.
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
    fn grouped_eql_values(&self, group_by: &'ast GroupByExpr) -> Vec<Option<EqlValue>> {
        match group_by {
            GroupByExpr::Expressions(exprs, _) => {
                exprs.iter().map(|expr| self.eql_value_of(expr)).collect()
            }
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

    /// Whether `item` projects one of the grouped encrypted columns, and so must
    /// be lifted through an aggregate.
    ///
    /// Matched on the resolved column rather than on syntax, so `SELECT t.col …
    /// GROUP BY col` — which PostgreSQL accepts today — keeps working.
    fn projects_grouped_column(&self, item: &'ast SelectItem, grouped: &[EqlValue]) -> bool {
        Self::select_item_expr(item)
            .and_then(|expr| self.eql_value_of(expr))
            .is_some_and(|value| grouped.contains(&value))
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

        let grouped = self.grouped_eql_values(&original.group_by);
        if grouped.iter().all(Option::is_none) {
            return Ok(false);
        }

        let Some(target) = target_node.downcast_mut::<Select>() else {
            return Ok(false);
        };

        // Group by the equality term.
        if let GroupByExpr::Expressions(exprs, _) = &mut target.group_by {
            for (expr, eql_value) in exprs.iter_mut().zip(grouped.iter()) {
                let Some(eql_value) = eql_value else { continue };

                let identity = eql_value.domain_identity();
                let Some(term_fn) = identity.eq_term_fn() else {
                    return Err(EqlMapperError::Transform(format!(
                        "encrypted column {} cannot be used in GROUP BY (domain {} carries no equality term)",
                        identity.token, identity.domain.value
                    )));
                };

                let grouped_expr = mem::replace(
                    expr,
                    Expr::Value(ValueWithSpan {
                        value: SqltkValue::Null,
                        span: Span::empty(),
                    }),
                );
                *expr = eql_v3_term_call(term_fn, grouped_expr);
            }
        }

        // Lift any projection of a grouped column through `any_value`, keeping
        // the name the client asked for.
        let grouped: Vec<EqlValue> = grouped.into_iter().flatten().collect();
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
                .grouped_eql_values(&original.group_by)
                .iter()
                .any(Option::is_some),
            None => false,
        }
    }
}
