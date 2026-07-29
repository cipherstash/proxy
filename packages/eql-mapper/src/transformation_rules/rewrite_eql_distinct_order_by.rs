use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::helpers::attached_token::AttachedToken;
use sqltk::parser::ast::{
    Distinct, Expr, GroupByExpr, Ident, OrderBy, OrderByKind, Query, Select, SelectFlavor,
    SelectItem, SetExpr, TableAlias, TableFactor, TableWithJoins, Value as SqltkValue, Values,
};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{Type, Value};
use crate::EqlMapperError;

use super::preserve_effective_aliases::derive_effective_alias;
use super::TransformationRule;

/// Alias given to the wrapping subquery.
const SUBQUERY_ALIAS: &str = "__eql_distinct";

/// Prefix for the synthetic name given to each projected column of the subquery.
const COLUMN_PREFIX: &str = "__eql_col_";

/// Prefix for the synthetic name given to each hoisted ordering term.
const ORDER_PREFIX: &str = "__eql_ord_";

/// The name PostgreSQL displays for a projection column that has no derivable
/// alias. Re-applied on the outer projection so wrapping does not rename a
/// column the client was already seeing as `?column?`.
const ANONYMOUS_COLUMN: &str = "?column?";

/// Rewrites `SELECT DISTINCT … ORDER BY <encrypted column>` by pushing the
/// select into a subquery that also projects the ordering term, and ordering the
/// outer query by that term:
///
/// ```sql
/// SELECT DISTINCT a, enc FROM t ORDER BY enc
/// -- becomes
/// SELECT __eql_col_0 AS a, __eql_col_1 AS enc
/// FROM (
///     SELECT DISTINCT a AS __eql_col_0, enc AS __eql_col_1,
///            eql_v3.ord_term(enc) AS __eql_ord_0
///     FROM t
/// ) AS __eql_distinct
/// ORDER BY __eql_ord_0
/// ```
///
/// # Why this is needed
///
/// [`super::RewriteEqlOrderBy`] must replace `ORDER BY enc` with
/// `ORDER BY eql_v3.ord_term(enc)` — ordering on the bare column compares whole
/// jsonb payloads starting at the randomised ciphertext, which is silently
/// wrong. But PostgreSQL requires that under `SELECT DISTINCT` every `ORDER BY`
/// expression also appear in the select list, and the ordering term does not:
///
/// ```text
/// ERROR: for SELECT DISTINCT, ORDER BY expressions must appear in select list
/// ```
///
/// So the two rewrites are individually correct and jointly invalid, and the
/// query has to be restructured. The outer query is not `DISTINCT`, so its
/// `ORDER BY` is free to reference a subquery column it does not project — which
/// is what lets the ordering term stay out of the client's result set.
///
/// # Why `DISTINCT` still means the same thing
///
/// Adding the ordering term to the `DISTINCT` list cannot change which rows are
/// distinct. `ord_term(enc)` is a deterministic function of `enc`, and `enc` is
/// itself in the list: any two rows agreeing on every original column agree on
/// `enc`, hence on `ord_term(enc)`. The extra column can only ever tie where the
/// originals tie.
///
/// # Naming
///
/// Every projected column is given a synthetic `__eql_col_N` name inside the
/// subquery and re-aliased to its original effective name on the way out. Going
/// through synthetic names (rather than referencing the original ones) keeps the
/// rewrite correct for a projection that names the same column twice, which
/// would otherwise make the outer reference ambiguous.
#[derive(Debug)]
pub struct RewriteEqlDistinctOrderBy<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

/// How an `ORDER BY` expression is carried out to the wrapping query.
enum OrderSource {
    /// An encrypted ordering term, hoisted into the subquery under this name.
    Hoisted(Ident),
    /// A reference to a projected column, by its synthetic subquery name.
    Column(Ident),
    /// An ordinal (`ORDER BY 2`). The outer projection preserves column order,
    /// so the ordinal is still correct and is left untouched.
    Ordinal,
}

impl<'ast> RewriteEqlDistinctOrderBy<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    fn is_eql(&self, expr: &'ast Expr) -> bool {
        matches!(
            self.node_types.get(&NodeKey::new(expr)),
            Some(Type::Value(Value::Eql(_)))
        )
    }

    /// The `SELECT` body of a query, if it has one.
    fn select_of(query: &'ast Query) -> Option<&'ast Select> {
        match query.body.as_ref() {
            SetExpr::Select(select) => Some(select),
            _ => None,
        }
    }

    /// The `ORDER BY` expressions of a query, if it orders by an expression list.
    fn order_by_exprs(query: &'ast Query) -> Option<&'ast Vec<sqltk::parser::ast::OrderByExpr>> {
        match &query.order_by {
            Some(OrderBy {
                kind: OrderByKind::Expressions(exprs),
                ..
            }) => Some(exprs),
            _ => None,
        }
    }

    /// The expression a select item projects, if it is a plain one.
    fn select_item_expr(item: &'ast SelectItem) -> Option<&'ast Expr> {
        match item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => Some(expr),
            _ => None,
        }
    }

    /// Whether this `SELECT DISTINCT … ORDER BY …` needs restructuring.
    ///
    /// Either half of the query can force it:
    ///
    /// - the `ORDER BY` names an encrypted column, so it becomes an ordering
    ///   term that `DISTINCT` will not accept outside the select list; or
    /// - the projection contains an encrypted column, so
    ///   [`super::RewriteEqlDistinct`] turns the `DISTINCT` into a `DISTINCT ON`
    ///   — and PostgreSQL then demands that the `ORDER BY` *begin with* the
    ///   `DISTINCT ON` expressions, which an arbitrary `ORDER BY` will not.
    ///
    /// Wrapping settles both: the subquery keeps the `DISTINCT`/`DISTINCT ON`
    /// with no `ORDER BY` to align with, and the outer query keeps the
    /// `ORDER BY` with no `DISTINCT` to constrain it.
    fn applies_to(&self, query: &'ast Query) -> bool {
        let (Some(select), Some(order_by)) = (Self::select_of(query), Self::order_by_exprs(query))
        else {
            return false;
        };

        if select.distinct.is_none() {
            return false;
        }

        let orders_by_encrypted = order_by.iter().any(|obe| self.is_eql(&obe.expr));

        let dedupes_encrypted = select
            .projection
            .iter()
            .filter_map(Self::select_item_expr)
            .any(|expr| self.is_eql(expr));

        orders_by_encrypted || dedupes_encrypted
    }

    /// Works out, for each `ORDER BY` expression, how the wrapping query will
    /// refer to it. Errors on the shapes this rewrite cannot express.
    fn plan_order_sources(
        &self,
        select: &'ast Select,
        order_by: &'ast [sqltk::parser::ast::OrderByExpr],
    ) -> Result<Vec<OrderSource>, EqlMapperError> {
        // `DISTINCT ON` constrains ORDER BY to *begin* with the ON expressions,
        // an invariant that wrapping would silently break.
        if let Some(Distinct::On(_)) = &select.distinct {
            return Err(EqlMapperError::Transform(
                "SELECT DISTINCT ON (…) cannot be combined with ORDER BY on an encrypted column: \
                 ordering requires the column's ordering term, which DISTINCT ON cannot carry"
                    .to_string(),
            ));
        }

        // A wildcard cannot be given a name, so the wrapping projection cannot
        // reproduce it column for column.
        if !select
            .projection
            .iter()
            .all(|item| Self::select_item_expr(item).is_some())
        {
            return Err(EqlMapperError::Transform(
                "SELECT DISTINCT with a wildcard cannot be combined with ORDER BY on an encrypted \
                 column: list the columns explicitly so the ordering term can be projected \
                 separately"
                    .to_string(),
            ));
        }

        let mut hoisted = 0usize;

        order_by
            .iter()
            .map(|obe| {
                if self.is_eql(&obe.expr) {
                    let ident = Ident::new(format!("{ORDER_PREFIX}{hoisted}"));
                    hoisted += 1;
                    return Ok(OrderSource::Hoisted(ident));
                }

                // An ordinal keeps working: the outer projection preserves both
                // the order and the count of the columns.
                if matches!(&obe.expr, Expr::Value(v) if matches!(v.value, SqltkValue::Number(_, _)))
                {
                    return Ok(OrderSource::Ordinal);
                }

                // Anything else must already be in the select list — PostgreSQL
                // requires it under DISTINCT — so point at that column.
                select
                    .projection
                    .iter()
                    .position(|item| Self::select_item_expr(item) == Some(&obe.expr))
                    .map(|idx| OrderSource::Column(Ident::new(format!("{COLUMN_PREFIX}{idx}"))))
                    .ok_or_else(|| {
                        EqlMapperError::Transform(format!(
                            "ORDER BY {} is not in the SELECT DISTINCT list, so it cannot be \
                             carried through the subquery required to order by an encrypted column",
                            obe.expr
                        ))
                    })
            })
            .collect()
    }
}

impl<'ast> TransformationRule<'ast> for RewriteEqlDistinctOrderBy<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Read the shape from the ORIGINAL query — `node_types` is keyed by it,
        // and the target's children are already rewritten.
        let Some((original,)) = node_path.last_1_as::<Query>() else {
            return Ok(false);
        };

        if !self.applies_to(original) {
            return Ok(false);
        }

        let (Some(original_select), Some(original_order_by)) =
            (Self::select_of(original), Self::order_by_exprs(original))
        else {
            return Ok(false);
        };

        let sources = self.plan_order_sources(original_select, original_order_by)?;

        let Some(target) = target_node.downcast_mut::<Query>() else {
            return Ok(false);
        };

        // Check the shape BEFORE moving anything: past this point the body has
        // been swapped out for a placeholder, so bailing with `Ok(false)` would
        // leave the caller holding a query rewritten into `VALUES ()` with its
        // original body dropped. `applies_to` already established this on the
        // original, so a mismatch here is a bug rather than an unhandled shape.
        if !matches!(target.body.as_ref(), SetExpr::Select(_)) {
            return Ok(false);
        }

        // Move the rewritten body out; it becomes the subquery.
        let body = mem::replace(
            &mut target.body,
            Box::new(SetExpr::Values(Values {
                explicit_row: false,
                rows: vec![],
            })),
        );

        let SetExpr::Select(mut inner_select) = *body else {
            return Err(EqlMapperError::InternalError(
                "SELECT DISTINCT rewrite: query body was a SELECT when checked but not when moved"
                    .to_string(),
            ));
        };

        // Give every projected column a synthetic name inside the subquery, and
        // rebuild the client-visible projection on the outside from the original
        // effective aliases.
        let mut outer_projection = Vec::with_capacity(inner_select.projection.len());

        for (idx, (original_item, inner_item)) in original_select
            .projection
            .iter()
            .zip(inner_select.projection.iter_mut())
            .enumerate()
        {
            let inner_alias = Ident::new(format!("{COLUMN_PREFIX}{idx}"));

            let expr =
                match inner_item {
                    SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                        mem::replace(expr, Expr::Value(SqltkValue::Null.into()))
                    }
                    // `plan_order_sources` has already rejected wildcards, and the
                    // body has been moved by now — erroring rather than bailing
                    // keeps a corrupted statement from reaching PostgreSQL.
                    _ => return Err(EqlMapperError::InternalError(
                        "SELECT DISTINCT rewrite: wildcard projection survived the wildcard check"
                            .to_string(),
                    )),
                };

            *inner_item = SelectItem::ExprWithAlias {
                expr,
                alias: inner_alias.clone(),
            };

            outer_projection.push(SelectItem::ExprWithAlias {
                expr: Expr::Identifier(inner_alias),
                alias: derive_effective_alias(original_item)
                    .unwrap_or_else(|| Ident::with_quote('"', ANONYMOUS_COLUMN)),
            });
        }

        // Hoist each encrypted ordering term into the subquery, and repoint the
        // outer ORDER BY at it.
        if let Some(OrderBy {
            kind: OrderByKind::Expressions(order_by),
            ..
        }) = &mut target.order_by
        {
            for (obe, source) in order_by.iter_mut().zip(sources.iter()) {
                match source {
                    OrderSource::Hoisted(alias) => {
                        let term =
                            mem::replace(&mut obe.expr, Expr::Value(SqltkValue::Null.into()));
                        inner_select.projection.push(SelectItem::ExprWithAlias {
                            expr: term,
                            alias: alias.clone(),
                        });
                        obe.expr = Expr::Identifier(alias.clone());
                    }
                    OrderSource::Column(alias) => {
                        obe.expr = Expr::Identifier(alias.clone());
                    }
                    OrderSource::Ordinal => {}
                }
            }
        }

        // The subquery carries the SELECT alone: ORDER BY, LIMIT, OFFSET, locks
        // and CTEs all stay on the wrapping query, so they still apply to the
        // ordered result.
        let subquery = Query {
            with: None,
            body: Box::new(SetExpr::Select(inner_select)),
            order_by: None,
            limit_clause: None,
            fetch: None,
            locks: vec![],
            for_clause: None,
            settings: None,
            format_clause: None,
            pipe_operators: vec![],
        };

        *target.body = SetExpr::Select(Box::new(Select {
            select_token: AttachedToken::empty(),
            distinct: None,
            top: None,
            top_before_distinct: false,
            projection: outer_projection,
            into: None,
            from: vec![TableWithJoins {
                relation: TableFactor::Derived {
                    lateral: false,
                    subquery: Box::new(subquery),
                    alias: Some(TableAlias {
                        name: Ident::new(SUBQUERY_ALIAS),
                        columns: vec![],
                    }),
                },
                joins: vec![],
            }],
            lateral_views: vec![],
            prewhere: None,
            selection: None,
            group_by: GroupByExpr::Expressions(vec![], vec![]),
            cluster_by: vec![],
            distribute_by: vec![],
            sort_by: vec![],
            having: None,
            named_window: vec![],
            qualify: None,
            window_before_qualify: false,
            value_table_mode: None,
            connect_by: None,
            flavor: SelectFlavor::Standard,
        }));

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        // Returning `true` for the shapes `plan_order_sources` rejects is
        // deliberate: it forces the caller into `apply`, which propagates the
        // error rather than silently skipping the rewrite.
        match node_path.last_1_as::<Query>() {
            Some((original,)) => self.applies_to(original),
            None => false,
        }
    }
}
