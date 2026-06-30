//! Concrete encrypted-index resolution for the SQL *transformation* stage.
//!
//! Type inference and unification only ever see the abstract [`crate::EqlTraits`]
//! (Eq / Ord / TokenMatch / JsonLike / Contain) of an encrypted column — never
//! the concrete index type that backs it. Trait-level information is sufficient
//! to decide *whether* an operation type-checks, but the concrete index decides
//! *which* function or operator form a transformation rule should emit.
//!
//! [`IndexResolver`] provides that concrete information to transformation rules
//! as a side-channel `(table, column) -> {IndexKind}` lookup. It is deliberately
//! kept out of the unifier and the [`crate::EqlValue`] / [`crate::EqlTerm`]
//! output types so that inference remains unchanged and `eql-mapper` does not
//! depend on `cipherstash-config`.

use std::collections::{HashMap, HashSet};

use crate::TableColumn;

/// A concrete encrypted index kind, mirroring the index families that
/// `cipherstash-config` describes for an encrypted column.
///
/// This is an `eql-mapper`-local representation so the crate does not depend on
/// `cipherstash-config`. It intentionally drops index *parameters* (tokenizer
/// settings, ste-vec prefix, …): transformation rules choose a target function
/// based only on which index *family* is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IndexKind {
    /// Order-revealing encryption (block ORE) — ordering via the root `ob` term.
    Ore,
    /// Order-preserving encryption — ordering via byte comparison of the `op` term.
    Ope,
    /// Structured encryption vector (jsonb) — ordering/containment over sv elements.
    SteVec,
    /// Bloom-filter match index — `LIKE` / `ILIKE`.
    Match,
    /// Deterministic (hmac) equality index.
    Unique,
}

/// Resolves the set of concrete [`IndexKind`]s configured for a `(table, column)`.
///
/// Transformation rules consult this to pick an index-specific target. The
/// resolver is a side-channel: it does not participate in inference or
/// unification.
///
/// A resolver that returns an empty set for every column (see
/// [`EmptyIndexResolver`]) reproduces the behaviour of rules that are not yet
/// concrete-index-aware, so it is the safe default.
pub trait IndexResolver: std::fmt::Debug + Send + Sync {
    /// Returns the set of [`IndexKind`]s configured for `table_column`.
    ///
    /// Returns an empty set when the column is unknown to the resolver (e.g. the
    /// encrypt config has not loaded it yet). Rules MUST treat an empty set as
    /// "no concrete information" and fall back to their default behaviour.
    fn resolve(&self, table_column: &TableColumn) -> HashSet<IndexKind>;
}

/// An [`IndexResolver`] that knows nothing: it returns an empty set for every
/// column. This is the default used by [`crate::type_check`] and reproduces the
/// pre-resolver transformation behaviour.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyIndexResolver;

impl IndexResolver for EmptyIndexResolver {
    fn resolve(&self, _table_column: &TableColumn) -> HashSet<IndexKind> {
        HashSet::new()
    }
}

/// An [`IndexResolver`] backed by an in-memory map. Primarily a building block
/// for callers (and tests) that already have a `(table, column) -> indexes`
/// mapping in hand.
#[derive(Debug, Default, Clone)]
pub struct MapIndexResolver {
    indexes: HashMap<TableColumn, HashSet<IndexKind>>,
}

impl MapIndexResolver {
    /// Creates a resolver from a `(table, column) -> {IndexKind}` map.
    pub fn new(indexes: HashMap<TableColumn, HashSet<IndexKind>>) -> Self {
        Self { indexes }
    }
}

impl IndexResolver for MapIndexResolver {
    fn resolve(&self, table_column: &TableColumn) -> HashSet<IndexKind> {
        self.indexes.get(table_column).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TableColumn;
    use sqltk::parser::ast::Ident;

    fn tc(table: &str, column: &str) -> TableColumn {
        TableColumn {
            table: Ident::new(table),
            column: Ident::new(column),
        }
    }

    #[test]
    fn empty_resolver_returns_empty_set_for_any_column() {
        let resolver = EmptyIndexResolver;
        assert_eq!(
            resolver.resolve(&tc("users", "email")),
            HashSet::new(),
            "empty resolver must return no index kinds"
        );
    }

    #[test]
    fn map_resolver_returns_configured_kinds_for_known_column() {
        let resolver = MapIndexResolver::new(HashMap::from_iter([(
            tc("users", "salary"),
            HashSet::from_iter([IndexKind::Ope]),
        )]));

        assert_eq!(
            resolver.resolve(&tc("users", "salary")),
            HashSet::from_iter([IndexKind::Ope])
        );
    }

    #[test]
    fn map_resolver_returns_full_set_for_multi_index_column() {
        let resolver = MapIndexResolver::new(HashMap::from_iter([(
            tc("users", "email"),
            HashSet::from_iter([IndexKind::Unique, IndexKind::Match, IndexKind::Ore]),
        )]));

        assert_eq!(
            resolver.resolve(&tc("users", "email")),
            HashSet::from_iter([IndexKind::Unique, IndexKind::Match, IndexKind::Ore])
        );
    }

    #[test]
    fn map_resolver_returns_empty_set_for_unknown_column() {
        let resolver = MapIndexResolver::new(HashMap::from_iter([(
            tc("users", "email"),
            HashSet::from_iter([IndexKind::Unique]),
        )]));

        assert_eq!(
            resolver.resolve(&tc("users", "unknown_column")),
            HashSet::new(),
            "unknown column must resolve to an empty set"
        );
    }
}
