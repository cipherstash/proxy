use std::{mem};

use sqlparser::ast::{Expr, Function, Ident, Select, SelectItem};
use sqltk::{Context, Visitable};

use crate::EqlMapperError;

use super::selector::{MatchTrailing, Selector};
use super::Rule;

/// [`Rule`] that ensures that a [`SelectItem`] has the same *effective* alias after EQL mapping that it had before EQL
/// mapping primarily so that we do not break existing database clients (e.g. ORMs) which expect specific columns to be
/// returned, but also we need to not break expected projection column names between outer queries and subqueries.
///
/// # Definitions:
///
/// This rule makes changes to the AST such that for all projection columns (including those of subqueries and
/// `RETURNING` clauses) the following invariant will be maintained:
///
/// effective_alias(col_before_mapping) == effective_alias(col_after_mapping)
///
/// # Determining the effective alias of a projection column
///
/// These rules were derived from reverse engineering what Postgres does. If we do not replicate what PG does consider
/// it a bug. Note where an effective alias is `None`, Postgres (via `psql` at least) would display `?column?` in that
/// situation.
///
/// 1. If old_col already has an explicit alias then that *is* the effective alias of old_col.
/// 2. If old_col has no explicit alias, then we attempt to emulate Postgres's algorithm for deriving an effective
///    alias:
///    - If the expression in old_col is an `Expr::Identifer(ident)` then `ident` becomes the effective identifier.
///    - If the expression in old_col is an `Expr::CompoundIdentifer(object_name)` then the last `Ident` element of
///      `object_name` becomes the effective identifier.
///    - If the expression in old_col is a `Function` then the name of the function becomes the effective identifier.
///    - If the expression in old_col is `Expr::Nested`, it is recursed (repeating all steps of 2.)
///    - If the expression in old_col is anything else then the effective alias is `None`
#[derive(Debug)]
pub struct PreserveAliases;

impl<'ast> Rule<'ast> for PreserveAliases {
    fn apply<N: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
    ) -> Result<N, EqlMapperError> {
        MatchTrailing::<(Select, Vec<SelectItem>, SelectItem)>::on_match_then(
            ctx,
            source_node,
            target_node,
            &mut |(_select, _select_items, select_item), mut target_node| {
                self.preserve_effective_alias_of_select_item(select_item, &mut target_node);
                Ok(target_node)
            },
        )
    }
}

impl PreserveAliases {
    fn preserve_effective_alias_of_select_item(
        &self,
        source_node: &SelectItem,
        target_node: &mut SelectItem,
    ) {
        let effective_source_alias = self.derive_effective_alias(source_node);
        let effective_target_alias = self.derive_effective_alias(target_node);
        match target_node {
            // The captured binding `expr` has type `&mut Expr` but we need an owned `Expr`.  to avoid cloning `expr`
            // (which can be arbitrarily large) we replace it with another which in return provides us with ownership of
            // the original value. `Expr::Wildcard` is chosen as the throwaway value because it's cheap.
            SelectItem::UnnamedExpr(expr) => {
                if let (Some(effective_target_alias), Some(effective_source_alias)) = (effective_target_alias, effective_source_alias)
                {
                    if effective_target_alias != effective_source_alias {
                        *target_node = SelectItem::ExprWithAlias {
                            expr: mem::replace(expr, Expr::Wildcard),
                            alias: effective_source_alias,
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn derive_effective_alias<'ast>(&self, node: &'ast SelectItem) -> Option<Ident> {
        match node {
            SelectItem::UnnamedExpr(expr) => self.derive_effective_alias_for_expr(expr),
            SelectItem::ExprWithAlias { expr: _, alias } => Some(alias.clone()),
            _ => None,
        }
    }

    fn derive_effective_alias_for_expr<'ast>(&self, expr: &'ast Expr) -> Option<Ident> {
        match expr {
            Expr::Identifier(ident) => Some(ident.clone()),
            Expr::CompoundIdentifier(idents) => Some(idents.last().unwrap().clone()),
            Expr::Function(Function { name, .. }) => Some(name.0.last().unwrap().clone()),
            Expr::Nested(expr) => self.derive_effective_alias_for_expr(&*expr),
            _ => None
        }
    }
}
