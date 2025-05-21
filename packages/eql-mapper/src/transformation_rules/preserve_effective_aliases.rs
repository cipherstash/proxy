use std::mem;

use sqltk::parser::ast::{
    helpers::attached_token::AttachedToken, Expr, Function, Ident, Select, SelectItem,
};
use sqltk::parser::tokenizer::{Span, Token, TokenWithSpan};
use sqltk::{NodePath, Visitable};

use crate::EqlMapperError;

use super::TransformationRule;

/// Ensures that a [`SelectItem`] has the same *effective* alias after EQL mapping that it had before EQL mapping
/// primarily so that we do not break existing database clients (e.g. ORMs) which expect specific columns to be
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
pub struct PreserveEffectiveAliases;

impl<'ast> TransformationRule<'ast> for PreserveEffectiveAliases {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((_select, _select_items, select_item)) =
                node_path.last_3_as::<Select, Vec<SelectItem>, SelectItem>()
            {
                let target_node = target_node.downcast_mut::<SelectItem>().unwrap();
                return Ok(Self::preserve_effective_alias_of_select_item(
                    select_item,
                    target_node,
                ));
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, target_node: &N) -> bool {
        if let Some((_select, _select_items, select_item)) =
            node_path.last_3_as::<Select, Vec<SelectItem>, SelectItem>()
        {
            let target_node = target_node.downcast_ref::<SelectItem>().unwrap();
            return Self::effective_aliases_differ(select_item, target_node);
        }
        false
    }
}

impl PreserveEffectiveAliases {
    fn effective_aliases_differ(source_node: &SelectItem, target_node: &SelectItem) -> bool {
        let effective_source_alias = Self::derive_effective_alias(source_node);
        let effective_target_alias = Self::derive_effective_alias(target_node);

        // The captured binding `expr` has type `&mut Expr` but we need an owned `Expr`.  to avoid cloning `expr`
        // (which can be arbitrarily large) we replace it with another which in return provides us with ownership of
        // the original value. `Expr::Wildcard` is chosen as the throwaway value because it's cheap.
        if let SelectItem::UnnamedExpr(_) = target_node {
            if let (Some(effective_target_alias), Some(effective_source_alias)) =
                (effective_target_alias, effective_source_alias)
            {
                return effective_target_alias != effective_source_alias;
            }
        }

        false
    }

    fn preserve_effective_alias_of_select_item(
        source_node: &SelectItem,
        target_node: &mut SelectItem,
    ) -> bool {
        let effective_source_alias = Self::derive_effective_alias(source_node);
        let effective_target_alias = Self::derive_effective_alias(target_node);

        // The captured binding `expr` has type `&mut Expr` but we need an owned `Expr`.  to avoid cloning `expr`
        // (which can be arbitrarily large) we replace it with another which in return provides us with ownership of
        // the original value. `Expr::Wildcard` is chosen as the throwaway value because it's cheap.
        if let SelectItem::UnnamedExpr(expr) = target_node {
            if let (Some(effective_target_alias), Some(effective_source_alias)) =
                (effective_target_alias, effective_source_alias)
            {
                if effective_target_alias != effective_source_alias {
                    *target_node = SelectItem::ExprWithAlias {
                        expr: mem::replace(
                            expr,
                            Expr::Wildcard(AttachedToken(TokenWithSpan::new(
                                Token::EOF,
                                Span::empty(),
                            ))),
                        ),
                        alias: effective_source_alias,
                    };

                    return true;
                }
            }
        }

        false
    }

    fn derive_effective_alias(node: &SelectItem) -> Option<Ident> {
        match node {
            SelectItem::UnnamedExpr(expr) => Self::derive_effective_alias_for_expr(expr),
            SelectItem::ExprWithAlias { expr: _, alias } => Some(alias.clone()),
            _ => None,
        }
    }

    fn derive_effective_alias_for_expr(expr: &Expr) -> Option<Ident> {
        match expr {
            Expr::Identifier(ident) => Some(ident.clone()),
            Expr::CompoundIdentifier(idents) => Some(idents.last().unwrap().clone()),
            Expr::Function(Function { name, .. }) => Some(name.0.last().unwrap().clone()),
            Expr::Nested(expr) => Self::derive_effective_alias_for_expr(expr),
            _ => None,
        }
    }
}
