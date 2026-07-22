//! Derives the proxy's encrypt configuration from the EQL v3 schema.
//!
//! EQL v3 domain types are self-configuring: a column's Postgres domain (e.g.
//! `eql_v3_text_search`) encodes both the plaintext token type and the
//! searchable-encryption terms it stores. That is enough to build the
//! [`ColumnConfig`] the encrypt pipeline needs, so `eql_v2.add_search_config`
//! and the `eql_v2_configuration` table are redundant.
//!
//! The SEM term → index mapping mirrors the client's indexers (verified against
//! `cipherstash-client`'s encrypt pipeline and `eql-bindings`' `v3::terms`):
//!
//! | term | produced by | index (`add_search_config` name) |
//! |------|-------------|-----------|
//! | `hm` (HMAC)      | `UniqueIndexer` | `Unique` (unique) |
//! | `op` (CLLW-OPE)  | `OpeIndexer`    | `Ope` (ope) |
//! | `ob` (block-ORE) | `OreIndexer`    | `Ore` (ore) |
//! | `bf` (bloom)     | `MatchIndexer`  | `Match` (match) |
//! | (SteVec/JSON)    | `JsonIndexer`   | `SteVec` (ste_vec) |

use cipherstash_client::schema::ColumnConfig;
use cipherstash_config::column::{ArrayIndexMode, Index, IndexType, SteVecMode};
use cipherstash_config::ColumnType;
use eql_mapper::{DomainIdentity, TokenType};

/// Build the [`ColumnConfig`] for a column from its EQL v3 domain typname, or
/// `None` if `domain` is not a recognised v3 EQL domain (a plaintext column).
pub(crate) fn column_config_from_domain(
    table: &str,
    column: &str,
    domain: &str,
) -> Option<ColumnConfig> {
    let identity = DomainIdentity::from_domain_name(domain)?;
    let mut config =
        ColumnConfig::build(column.to_string()).casts_as(token_to_column_type(identity.token));

    if identity.token == TokenType::Json {
        // Searchable encrypted JSON is a SteVec index; its terms live per entry,
        // not on the domain, so the scalar term flags below do not apply. A
        // storage-only `eql_v3_json` column has no searchable index.
        if domain.ends_with("_search") {
            config = config.add_index(Index::new(IndexType::SteVec {
                prefix: format!("{table}/{column}"),
                term_filters: Vec::new(),
                array_index_mode: ArrayIndexMode::default(),
                mode: SteVecMode::default(), // Compat (CLLW-OPE), the v3 default
            }));
        }
    } else {
        // Scalar domains: the stored terms map directly to index types.
        // `DomainIdentity::stores_*` already encodes the text `hm` exception.
        if identity.stores_hm() {
            config = config.add_index(Index::new_unique());
        }
        if identity.stores_op() {
            config = config.add_index(Index::new_ope());
        }
        if identity.stores_ob() {
            config = config.add_index(Index::new_ore());
        }
        if identity.stores_bf() {
            config = config.add_index(Index::new_match());
        }
    }

    Some(config)
}

fn token_to_column_type(token: TokenType) -> ColumnType {
    match token {
        TokenType::SmallInt => ColumnType::SmallInt,
        TokenType::Integer => ColumnType::Int,
        TokenType::BigInt => ColumnType::BigInt,
        TokenType::Real | TokenType::Double => ColumnType::Float,
        TokenType::Numeric => ColumnType::Decimal,
        TokenType::Text => ColumnType::Text,
        TokenType::Boolean => ColumnType::Boolean,
        TokenType::Date => ColumnType::Date,
        TokenType::Timestamp => ColumnType::Timestamp,
        TokenType::Json => ColumnType::Json,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn config(domain: &str) -> ColumnConfig {
        column_config_from_domain("t", "c", domain)
            .unwrap_or_else(|| panic!("{domain} did not resolve"))
    }

    fn index_types(domain: &str) -> Vec<IndexType> {
        config(domain)
            .indexes
            .into_iter()
            .map(|i| i.index_type)
            .collect()
    }

    fn has(domain: &str, matcher: impl Fn(&IndexType) -> bool) -> bool {
        index_types(domain).iter().any(matcher)
    }

    #[test]
    fn cast_type_comes_from_the_token() {
        assert_eq!(config("eql_v3_integer_eq").cast_type, ColumnType::Int);
        assert_eq!(config("eql_v3_bigint_eq").cast_type, ColumnType::BigInt);
        assert_eq!(config("eql_v3_smallint_eq").cast_type, ColumnType::SmallInt);
        assert_eq!(config("eql_v3_double_ord").cast_type, ColumnType::Float);
        assert_eq!(config("eql_v3_text_search").cast_type, ColumnType::Text);
        assert_eq!(config("eql_v3_boolean").cast_type, ColumnType::Boolean);
        assert_eq!(config("eql_v3_date_ord").cast_type, ColumnType::Date);
        assert_eq!(config("eql_v3_json_search").cast_type, ColumnType::Json);
    }

    #[test]
    fn eq_domain_has_a_unique_index() {
        assert_eq!(
            index_types("eql_v3_integer_eq"),
            vec![IndexType::Unique {
                token_filters: vec![]
            }]
        );
    }

    #[test]
    fn scalar_ord_uses_ope_only_no_hmac() {
        // A non-text `_ord` domain stores only `op` (no `hm`): a single Ope index.
        assert_eq!(index_types("eql_v3_integer_ord"), vec![IndexType::Ope]);
        // block-ORE ordering -> Ore.
        assert_eq!(index_types("eql_v3_integer_ord_ore"), vec![IndexType::Ore]);
    }

    #[test]
    fn text_ord_carries_unique_plus_ordering() {
        // text stores `hm` alongside its ordering term (equality-lossy ORE/OPE).
        assert!(has("eql_v3_text_ord", |i| matches!(
            i,
            IndexType::Unique { .. }
        )));
        assert!(has("eql_v3_text_ord", |i| *i == IndexType::Ope));
        assert!(has("eql_v3_text_ord_ore", |i| matches!(
            i,
            IndexType::Unique { .. }
        )));
        assert!(has("eql_v3_text_ord_ore", |i| *i == IndexType::Ore));
    }

    #[test]
    fn text_search_has_unique_ope_and_match() {
        assert!(has("eql_v3_text_search", |i| matches!(
            i,
            IndexType::Unique { .. }
        )));
        assert!(has("eql_v3_text_search", |i| *i == IndexType::Ope));
        assert!(has("eql_v3_text_search", |i| matches!(
            i,
            IndexType::Match { .. }
        )));
    }

    #[test]
    fn match_domain_has_a_match_index() {
        assert!(has("eql_v3_text_match", |i| matches!(
            i,
            IndexType::Match { .. }
        )));
        assert!(!has("eql_v3_text_match", |i| matches!(
            i,
            IndexType::Unique { .. }
        )));
    }

    #[test]
    fn json_search_has_a_ste_vec_index_in_compat_mode() {
        let idx = index_types("eql_v3_json_search");
        assert_eq!(idx.len(), 1);
        match &idx[0] {
            IndexType::SteVec { mode, .. } => assert_eq!(*mode, SteVecMode::Compat),
            other => panic!("expected SteVec, got {other:?}"),
        }
    }

    #[test]
    fn storage_only_domains_have_no_indexes() {
        assert!(config("eql_v3_integer").indexes.is_empty());
        assert!(config("eql_v3_boolean").indexes.is_empty());
        // storage-only json (no `_search`) is not searchable.
        assert!(config("eql_v3_json").indexes.is_empty());
    }

    #[test]
    fn non_eql_domains_do_not_resolve() {
        assert!(column_config_from_domain("t", "c", "jsonb").is_none());
        assert!(column_config_from_domain("t", "c", "text").is_none());
        assert!(column_config_from_domain("t", "c", "eql_v2_encrypted").is_none());
    }
}
