use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{Expr, OrderByExpr, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{DomainIdentity, Type, Value};
use crate::EqlMapperError;

use super::helpers::eql_v3_term_call;
use super::TransformationRule;

/// Rewrites `ORDER BY` on an encrypted column to order by its **ordering term**:
///
/// - `ORDER BY col`            → `ORDER BY eql_v3.ord_term(col)`
/// - `ORDER BY t.col DESC`     → `ORDER BY eql_v3.ord_term(t.col) DESC`
/// - `ORDER BY col NULLS FIRST`→ `ORDER BY eql_v3.ord_term(col) NULLS FIRST`
///
/// (`ord_term_ore` for block-ORE domains.) Sort options are untouched, so
/// `ASC`/`DESC` and `NULLS FIRST`/`LAST` keep working — the term is a plain
/// orderable value and `NULL` stays `NULL`.
///
/// **Without this rewrite the results are silently misordered.** An encrypted
/// column is a domain over `jsonb`, so a bare `ORDER BY` compares whole payloads
/// by jsonb rules — and jsonb compares objects field by field starting at `c`,
/// the ciphertext, which is randomised per encryption. The rows come back in an
/// order that is not just wrong but *different on every insert*. The ordering
/// terms exist precisely to avoid that: `ord_term` yields `ope_cllw` (a `bytea`
/// domain, ordered bytewise) and `ord_term_ore` yields `ore_block_256` (ordered
/// by its own btree operator class).
///
/// A column whose domain carries no ordering term cannot be ordered at all, and
/// is a capability error rather than a silent arbitrary sort.
#[derive(Debug)]
pub struct RewriteEqlOrderBy<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlOrderBy<'ast> {
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
}

impl<'ast> TransformationRule<'ast> for RewriteEqlOrderBy<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Read the identity from the ORIGINAL node — `node_types` is keyed by it.
        let Some((original,)) = node_path.last_1_as::<OrderByExpr>() else {
            return Ok(false);
        };

        let Some(identity) = self.eql_identity_of(&original.expr) else {
            return Ok(false);
        };

        let Some(term_fn) = identity.ord_term_fn() else {
            return Err(EqlMapperError::Transform(format!(
                "encrypted column {} cannot be used in ORDER BY (domain {} carries no ordering term)",
                identity.token, identity.domain.value
            )));
        };

        let Some(target) = target_node.downcast_mut::<OrderByExpr>() else {
            return Ok(false);
        };

        let expr = mem::replace(
            &mut target.expr,
            Expr::Value(ValueWithSpan {
                value: SqltkValue::Null,
                span: Span::empty(),
            }),
        );
        target.expr = eql_v3_term_call(term_fn, expr);

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        match node_path.last_1_as::<OrderByExpr>() {
            Some((original,)) => self.eql_identity_of(&original.expr).is_some(),
            None => false,
        }
    }
}
