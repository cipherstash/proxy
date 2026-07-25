//! The N:1 fusion record for encrypted-JSON equality.
//!
//! `col -> sel = value` does not compare two encrypted terms. Exact JSON
//! equality in EQL v3 is *containment of a value selector*: a single keyed MAC
//! over the path and the canonicalised value together
//! (`QueryOp::SteVecValueSelector`, input `{"path": <jsonpath>, "value":
//! <scalar>}`). One needle, built from **two** SQL operands.
//!
//! The mapper cannot build it — it holds no encryption key. So the mapper does
//! the half it can: it types the value operand [`EqlTerm::JsonValueSelector`],
//! drops the path operand from the rewritten SQL, and records *where the path
//! came from* so the proxy can fuse the pair at encryption time.
//!
//! [`EqlTerm::JsonValueSelector`]: crate::EqlTerm::JsonValueSelector

use std::collections::HashMap;

use sqltk::parser::ast;
use sqltk::NodeKey;

use crate::Param;

/// Where the JSON path half of a fused value selector comes from.
///
/// The two halves are independently a literal or a placeholder, so all four
/// combinations occur (`-> 'a' = '1'`, `-> $1 = $2`, `-> 'a' = $1`, …). A
/// literal path is fully known at type-check time and is carried inline; a
/// placeholder path is only known at Bind, so its param number is carried
/// instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonSelectorSource {
    /// A SQL literal path (`col -> 'name' = …`) — the selector text itself.
    Literal(String),

    /// A placeholder path (`col -> $1 = …`) — the param it will arrive in.
    Param(Param),
}

/// The set of fused JSON value selectors in a statement: for each operand that
/// carries the *value* half, where its *path* half comes from.
///
/// Keyed separately for the two protocols the proxy has to serve — params are
/// addressed by number (the extended protocol has no AST at Bind time),
/// literals by AST node.
#[derive(Debug, Default)]
pub struct JsonValueSelectors<'ast> {
    by_param: HashMap<Param, JsonSelectorSource>,
    by_literal: HashMap<NodeKey<'ast>, JsonSelectorSource>,
}

impl<'ast> JsonValueSelectors<'ast> {
    pub(crate) fn record_param(&mut self, param: Param, source: JsonSelectorSource) {
        self.by_param.insert(param, source);
    }

    pub(crate) fn record_literal(&mut self, node: &'ast ast::Value, source: JsonSelectorSource) {
        self.by_literal.insert(NodeKey::new(node), source);
    }

    /// The path source for the value-selector operand bound to `param`, or
    /// `None` if that param is not one.
    pub fn for_param(&self, param: Param) -> Option<&JsonSelectorSource> {
        self.by_param.get(&param)
    }

    /// The path source for the value-selector operand at literal `node`, or
    /// `None` if that literal is not one.
    pub fn for_literal(&self, node: &'ast ast::Value) -> Option<&JsonSelectorSource> {
        self.by_literal.get(&NodeKey::new(node))
    }

    pub fn is_empty(&self) -> bool {
        self.by_param.is_empty() && self.by_literal.is_empty()
    }
}
