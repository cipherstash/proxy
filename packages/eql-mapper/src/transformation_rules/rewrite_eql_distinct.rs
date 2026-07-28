use std::collections::HashMap;
use std::sync::Arc;

use sqltk::parser::ast::{Distinct, Expr, Select, SelectItem};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{EqlValue, Type, Value};
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
    fn select_item_expr(item: &'ast SelectItem) -> Option<&'ast Expr> {
        match item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => Some(expr),
            _ => None,
        }
    }

    /// The encrypted columns a plain `DISTINCT` would deduplicate on, positionally.
    fn distinct_eql_values(&self, select: &'ast Select) -> Vec<Option<EqlValue>> {
        if !matches!(select.distinct, Some(Distinct::Distinct)) {
            return vec![];
        }

        select
            .projection
            .iter()
            .map(|item| Self::select_item_expr(item).and_then(|expr| self.eql_value_of(expr)))
            .collect()
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

        let distinct = self.distinct_eql_values(original);
        if distinct.iter().all(Option::is_none) {
            return Ok(false);
        }

        // A wildcard hides the columns being deduplicated on, so the key list
        // cannot be built and the encrypted columns would dedupe on ciphertext.
        if original
            .projection
            .iter()
            .any(|item| Self::select_item_expr(item).is_none())
        {
            return Err(EqlMapperError::Transform(
                "SELECT DISTINCT with a wildcard cannot deduplicate an encrypted column: list the \
                 columns explicitly so each one can be keyed on its equality term"
                    .to_string(),
            ));
        }

        let Some(target) = target_node.downcast_mut::<Select>() else {
            return Ok(false);
        };

        let mut keys = Vec::with_capacity(target.projection.len());

        for (item, eql_value) in target.projection.iter().zip(distinct.iter()) {
            let Some(expr) = (match item {
                SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                    Some(expr)
                }
                _ => None,
            }) else {
                return Ok(false);
            };

            match eql_value {
                Some(eql_value) => {
                    let identity = eql_value.domain_identity();
                    let Some(term_fn) = identity.eq_term_fn() else {
                        return Err(EqlMapperError::Transform(format!(
                            "encrypted column {} cannot be used in SELECT DISTINCT (domain {} carries no equality term)",
                            identity.token, identity.domain.value
                        )));
                    };

                    keys.push(eql_v3_term_call(term_fn, expr.clone()));
                }
                // A plaintext column keys on itself.
                None => keys.push(expr.clone()),
            }
        }

        target.distinct = Some(Distinct::On(keys));

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        match node_path.last_1_as::<Select>() {
            Some((original,)) => self
                .distinct_eql_values(original)
                .iter()
                .any(Option::is_some),
            None => false,
        }
    }
}
