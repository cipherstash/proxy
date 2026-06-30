//! Proxy-side [`IndexResolver`] backed by the loaded [`EncryptConfig`].
//!
//! `eql-mapper`'s transformation stage asks for the concrete encrypted-index
//! kinds of a `(table, column)` pair so index-specific rewrite rules (e.g.
//! scalar OPE ordering) can target the right function. The concrete index types
//! already live in [`EncryptConfig`] (keyed by [`eql::Identifier`]); this
//! resolver translates a `cipherstash-config` [`IndexType`] into the
//! `eql-mapper`-local [`IndexKind`] and exposes them through the
//! [`IndexResolver`] trait.
//!
//! It owns an `Arc<EncryptConfig>` snapshot so a single statement sees a
//! consistent view of the config even if the background reloader swaps in a new
//! one mid-transform.

use std::collections::HashSet;
use std::sync::Arc;

use cipherstash_client::eql::Identifier;
use cipherstash_config::column::IndexType;
use eql_mapper::{IndexKind, IndexResolver, TableColumn};

use super::EncryptConfig;

/// Maps a `cipherstash-config` [`IndexType`] to the `eql-mapper`-local
/// [`IndexKind`]. Index parameters (tokenizer, ste-vec prefix, …) are dropped:
/// transformation rules only care about the index *family*.
fn index_kind_of(index_type: &IndexType) -> IndexKind {
    match index_type {
        IndexType::Ore => IndexKind::Ore,
        IndexType::Ope => IndexKind::Ope,
        IndexType::Match { .. } => IndexKind::Match,
        IndexType::Unique { .. } => IndexKind::Unique,
        IndexType::SteVec { .. } => IndexKind::SteVec,
    }
}

/// An [`IndexResolver`] backed by a snapshot of the [`EncryptConfig`].
#[derive(Debug)]
pub struct EncryptConfigIndexResolver {
    encrypt_config: Arc<EncryptConfig>,
}

impl EncryptConfigIndexResolver {
    pub fn new(encrypt_config: Arc<EncryptConfig>) -> Self {
        Self { encrypt_config }
    }
}

impl IndexResolver for EncryptConfigIndexResolver {
    fn resolve(&self, table_column: &TableColumn) -> HashSet<IndexKind> {
        // Mirror how the column mapper derives an Identifier from a TableColumn
        // (unquoted ident text) so the lookup keys match.
        let identifier = Identifier::new(
            table_column.table.value.to_string(),
            table_column.column.value.to_string(),
        );

        match self.encrypt_config.get_column_config(&identifier) {
            Some(config) => config
                .indexes
                .iter()
                .map(|index| index_kind_of(&index.index_type))
                .collect(),
            // Unknown column (e.g. encrypt config not yet loaded for it) → empty
            // set, so rules fall back to their default behaviour.
            None => HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::encrypt_config::EncryptConfig;
    use cipherstash_config::CanonicalEncryptionConfig;
    use eql_mapper::TableColumn;
    use serde_json::json;
    use sqltk::parser::ast::Ident;

    fn config_from(json: serde_json::Value) -> EncryptConfig {
        let canonical: CanonicalEncryptionConfig = serde_json::from_value(json).unwrap();
        let map = super::super::manager::canonical_to_map(canonical).unwrap();
        EncryptConfig::new_from_config(map)
    }

    fn table_column(table: &str, column: &str) -> TableColumn {
        TableColumn {
            table: Ident::new(table),
            column: Ident::new(column),
        }
    }

    #[test]
    fn resolves_ope_index_to_ope_kind() {
        let config = config_from(json!({
            "v": 1,
            "tables": { "users": { "salary": { "indexes": { "ope": {} } } } }
        }));

        let resolver = EncryptConfigIndexResolver::new(Arc::new(config));

        assert_eq!(
            resolver.resolve(&table_column("users", "salary")),
            HashSet::from_iter([IndexKind::Ope])
        );
    }

    #[test]
    fn resolves_ore_index_to_ore_kind() {
        let config = config_from(json!({
            "v": 1,
            "tables": { "users": { "salary": { "indexes": { "ore": {} } } } }
        }));

        let resolver = EncryptConfigIndexResolver::new(Arc::new(config));

        assert_eq!(
            resolver.resolve(&table_column("users", "salary")),
            HashSet::from_iter([IndexKind::Ore])
        );
    }

    #[test]
    fn resolves_multi_index_column_to_full_set() {
        let config = config_from(json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": { "indexes": { "unique": {}, "match": {}, "ore": {} } }
                }
            }
        }));

        let resolver = EncryptConfigIndexResolver::new(Arc::new(config));

        assert_eq!(
            resolver.resolve(&table_column("users", "email")),
            HashSet::from_iter([IndexKind::Unique, IndexKind::Match, IndexKind::Ore])
        );
    }

    #[test]
    fn resolves_ste_vec_index_to_ste_vec_kind() {
        let config = config_from(json!({
            "v": 1,
            "tables": {
                "users": {
                    "event_data": {
                        "cast_as": "jsonb",
                        "indexes": { "ste_vec": { "prefix": "event-data" } }
                    }
                }
            }
        }));

        let resolver = EncryptConfigIndexResolver::new(Arc::new(config));

        assert_eq!(
            resolver.resolve(&table_column("users", "event_data")),
            HashSet::from_iter([IndexKind::SteVec])
        );
    }

    #[test]
    fn unknown_column_resolves_to_empty_set() {
        let config = config_from(json!({
            "v": 1,
            "tables": { "users": { "salary": { "indexes": { "ope": {} } } } }
        }));

        let resolver = EncryptConfigIndexResolver::new(Arc::new(config));

        assert_eq!(
            resolver.resolve(&table_column("users", "unknown")),
            HashSet::new()
        );
    }

    #[test]
    fn empty_config_resolves_to_empty_set() {
        let resolver = EncryptConfigIndexResolver::new(Arc::new(EncryptConfig::new()));

        assert_eq!(
            resolver.resolve(&table_column("users", "salary")),
            HashSet::new()
        );
    }
}
