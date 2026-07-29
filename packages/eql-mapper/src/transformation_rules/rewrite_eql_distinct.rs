use std::collections::HashMap;
use std::sync::Arc;

use sqltk::parser::ast::{Distinct, Expr, Select, SelectItem};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{EqlValue, NativeValue, Projection, Type, Value};
use crate::EqlMapperError;

use super::helpers::eql_v3_term_call;
use super::TransformationRule;

/// Rewrites `SELECT DISTINCT` over an encrypted column to deduplicate on the
/// column's **equality term**:
///
/// ```sql
/// SELECT DISTINCT enc FROM t
/// -- becomes
/// SELECT DISTINCT ON (eql_v3.eq_term(enc)) enc FROM t
/// ```
///
/// **Without this rewrite the deduplication silently does nothing.** An
/// encrypted column is a domain over `jsonb`, so a bare `DISTINCT` compares
/// whole payloads — including `c`, the ciphertext, which is randomised per
/// encryption. Two rows holding the *same* plaintext have different ciphertexts,
/// so every row looks distinct and `DISTINCT` degrades into a no-op that quietly
/// returns duplicates.
///
/// Deduplication is equality, so the key is the same `eq_term` an `=` comparison
/// uses (`ord_term` for a domain that stores no `hm`). `DISTINCT ON` is what
/// lets the key differ from the projection: the column is still returned in
/// full — one row per distinct plaintext — while the grouping happens on the
/// term.
///
/// A plaintext column in the same projection keys on itself, so a mixed
/// `SELECT DISTINCT plain, enc` becomes
/// `SELECT DISTINCT ON (plain, eql_v3.eq_term(enc)) plain, enc`.
///
/// Which row of each group is returned is unspecified, which is fine: every row
/// in a group is an encryption of the same plaintext, so they all decrypt alike.
///
/// [`super::RewriteEqlDistinctOrderBy`] composes on top of this. It reads the
/// *original* query to decide whether to wrap, so it still sees a plain
/// `DISTINCT` here and is unaffected by the `DISTINCT ON` this produces; the
/// `DISTINCT ON` simply travels into the subquery it builds, where the absence
/// of an `ORDER BY` leaves PostgreSQL free to pick any row per group.
#[derive(Debug)]
pub struct RewriteEqlDistinct<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlDistinct<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    fn eql_value_of(&self, expr: &'ast Expr) -> Option<EqlValue> {
        match self.node_types.get(&NodeKey::new(expr)) {
            Some(Type::Value(Value::Eql(eql_term))) => Some(eql_term.eql_value().clone()),
            _ => None,
        }
    }

    /// The expression a select item projects, if it is a plain one.
    ///
    /// Elided lifetime: this is called on both the original (`'ast`) items and
    /// the shorter-lived rewritten ones.
    fn select_item_expr(item: &SelectItem) -> Option<&Expr> {
        match item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => Some(expr),
            _ => None,
        }
    }

    /// The columns a wildcard projects, as `(expression to key on, encrypted
    /// value if it is one)`.
    ///
    /// A wildcard carries no per-column expression, so the columns are read off
    /// the projection type it resolved to and named explicitly. `None` if any
    /// column has no name to write — a wildcard over a derived table's computed
    /// column, say — since the projection cannot then be reproduced.
    fn wildcard_columns(&self, item: &'ast SelectItem) -> Option<Vec<(Expr, Option<EqlValue>)>> {
        let Some(Type::Value(Value::Projection(projection))) =
            self.node_types.get(&NodeKey::new(item))
        else {
            return None;
        };

        let mut columns = Vec::new();
        Self::collect_columns(projection, &mut columns)?;
        Some(columns)
    }

    /// Flattens a projection into named columns.
    ///
    /// A wildcard's projection nests: one entry per relation in the `FROM`,
    /// each holding that relation's columns. Returns `None` if any leaf has no
    /// name to write.
    fn collect_columns(
        projection: &Projection,
        out: &mut Vec<(Expr, Option<EqlValue>)>,
    ) -> Option<()> {
        for column in projection.columns() {
            let (table_column, eql) = match &*column.ty {
                Type::Value(Value::Projection(nested)) => {
                    Self::collect_columns(nested, out)?;
                    continue;
                }
                Type::Value(Value::Eql(eql_term)) => (
                    eql_term.eql_value().table_column().clone(),
                    Some(eql_term.eql_value().clone()),
                ),
                Type::Value(Value::Native(NativeValue(Some(table_column)))) => {
                    (table_column.clone(), None)
                }
                _ => return None,
            };

            out.push((
                Expr::CompoundIdentifier(vec![
                    table_column.table.clone(),
                    table_column.column.clone(),
                ]),
                eql,
            ));
        }

        Some(())
    }

    /// Whether this `SELECT DISTINCT` deduplicates any encrypted column,
    /// including ones a wildcard hides.
    fn dedupes_encrypted(&self, select: &'ast Select) -> bool {
        if !matches!(select.distinct, Some(Distinct::Distinct)) {
            return false;
        }

        select.projection.iter().any(|item| {
            match Self::select_item_expr(item) {
                Some(expr) => self.eql_value_of(expr).is_some(),
                // A wildcard that cannot be resolved is reported in `apply`
                // rather than skipped here — treating it as "no encrypted
                // columns" is what let `SELECT DISTINCT *` slip through
                // unprotected.
                None => self
                    .wildcard_columns(item)
                    .is_none_or(|cols| cols.iter().any(|(_, eql)| eql.is_some())),
            }
        })
    }
}

impl<'ast> TransformationRule<'ast> for RewriteEqlDistinct<'ast> {
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

        if !self.dedupes_encrypted(original) {
            return Ok(false);
        }

        let Some(target) = target_node.downcast_mut::<Select>() else {
            return Ok(false);
        };

        // Build the projection plan: a plain item keys on its own (already
        // rewritten) expression; a wildcard contributes one entry per column it
        // resolves to, named explicitly so it can be keyed at all.
        let mut keys = Vec::with_capacity(target.projection.len());
        let mut projection = Vec::with_capacity(target.projection.len());
        let mut expanded_a_wildcard = false;

        let target_items = target.projection.clone();

        for (original_item, target_item) in original.projection.iter().zip(target_items.iter()) {
            let planned = match Self::select_item_expr(original_item) {
                Some(original_expr) => {
                    let Some(target_expr) = Self::select_item_expr(target_item) else {
                        return Ok(false);
                    };

                    projection.push(target_item.clone());
                    vec![(target_expr.clone(), self.eql_value_of(original_expr))]
                }

                None => {
                    // The wildcard has to be written out: `DISTINCT ON` keys are
                    // expressions, and `*` names nothing to key on. Expanding it
                    // projects the same columns in the same order, so the result
                    // the client sees is unchanged.
                    let Some(columns) = self.wildcard_columns(original_item) else {
                        return Err(EqlMapperError::Transform(
                            "SELECT DISTINCT with a wildcard cannot deduplicate an encrypted \
                             column: the wildcard's columns cannot be named, so list them \
                             explicitly to key each one on its equality term"
                                .to_string(),
                        ));
                    };

                    expanded_a_wildcard = true;
                    projection.extend(
                        columns
                            .iter()
                            .map(|(expr, _)| SelectItem::UnnamedExpr(expr.clone())),
                    );
                    columns
                }
            };

            for (expr, eql_value) in planned {
                match eql_value {
                    Some(eql_value) => {
                        let identity = eql_value.domain_identity();
                        let Some(term_fn) = identity.eq_term_fn() else {
                            return Err(EqlMapperError::Transform(format!(
                                "encrypted column {} cannot be used in SELECT DISTINCT (domain {} carries no equality term)",
                                identity.token, identity.domain.value
                            )));
                        };

                        keys.push(eql_v3_term_call(term_fn, expr));
                    }
                    // A plaintext column keys on itself.
                    None => keys.push(expr),
                }
            }
        }

        if expanded_a_wildcard {
            target.projection = projection;
        }

        target.distinct = Some(Distinct::On(keys));

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        match node_path.last_1_as::<Select>() {
            Some((original,)) => self.dedupes_encrypted(original),
            None => false,
        }
    }
}
