//! Which encrypted operands are **query operands** rather than stored values.
//!
//! An encrypted value reaches PostgreSQL in one of two shapes. A *stored value*
//! carries the column's whole payload — the ciphertext `c` plus every search
//! term. A *query operand* carries the terms but **never** a decryptable
//! ciphertext; the `eql_v3.query_*` domains enforce that with a `NOT (VALUE ?
//! 'c')` check.
//!
//! The two are indistinguishable by type: the operand of `WHERE col = $1` and
//! the value of `INSERT ... VALUES ($1)` are both [`crate::EqlTerm::Full`]. What
//! separates them is where they appear, which is what this records.
//!
//! The proxy needs it because it cannot encrypt a multi-term operand directly —
//! a single `EqlOperation::Query` yields only one term, so an operand is
//! encrypted in Store mode and then projected
//! (`EqlCiphertextV3::into_query_operand`). Without knowing the role it would
//! send a stored payload into a query position and PostgreSQL would reject it.

use std::collections::HashSet;

use sqltk::parser::ast;
use sqltk::NodeKey;

use crate::Param;

/// The set of operands that appear in a query position.
///
/// Membership is decided syntactically, by the predicate an operand belongs to
/// — the same contexts whose rewrite rules cast to a `eql_v3.query_*` twin:
/// comparisons (`=`, `<>`, `<`, `<=`, `>`, `>=`), containment (`@>`/`<@`),
/// `LIKE`/`ILIKE` and `@@`. Everything else — including `INSERT` values and
/// `UPDATE` assignments — is a stored value and keeps its full payload.
#[derive(Debug, Default)]
pub struct QueryOperands<'ast> {
    params: HashSet<Param>,
    literals: HashSet<NodeKey<'ast>>,
}

impl<'ast> QueryOperands<'ast> {
    pub(crate) fn record_param(&mut self, param: Param) {
        self.params.insert(param);
    }

    pub(crate) fn record_literal(&mut self, node: &'ast ast::Value) {
        self.literals.insert(NodeKey::new(node));
    }

    /// Whether the value bound to `param` is a query operand.
    pub fn contains_param(&self, param: Param) -> bool {
        self.params.contains(&param)
    }

    /// Whether the literal at `node` is a query operand.
    pub fn contains_literal(&self, node: &'ast ast::Value) -> bool {
        self.literals.contains(&NodeKey::new(node))
    }

    pub fn is_empty(&self) -> bool {
        self.params.is_empty() && self.literals.is_empty()
    }
}
