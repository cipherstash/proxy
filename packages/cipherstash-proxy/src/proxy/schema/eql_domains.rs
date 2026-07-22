//! Resolves EQL v3 Postgres domain typnames to the inert domain identity and
//! capabilities the type checker needs (ADR-0002).
//!
//! The mapping is inverted from the `eql-bindings` v3 catalog (`v3::all()`),
//! which is generated from the same source as the installed SQL domains, so it
//! cannot drift from them. `eql-mapper` stays wire-format-agnostic — this
//! `eql-bindings` dependency lives only in the proxy's schema loader.
//!
//! Term → capability mapping (from `eql-bindings` `v3::terms`):
//! - `hm` (HMAC-256)  → `Eq`
//! - `op` (CLLW-OPE)  → `Ord`
//! - `ob` (block-ORE) → `Ord`
//! - `bf` (bloom)     → `TokenMatch`
//! - `c` / empty      → storage-only (no capabilities)
//!
//! `term_json_keys() == None` marks the JSON SteVec domains
//! (`eql_v3_json_search`, `eql_v3_json_entry`), whose searchable terms live
//! per-entry rather than on the domain. Verified against the installed v3 SQL
//! (`cipherstash-encrypt.sql`), an encrypted JSON column supports `->`/`->>`
//! (JsonLike) **and** `@>`/`<@` containment (Contain) — the latter are real
//! implementations, not the raise-stubs the other jsonb operators get. So JSON
//! domains map to `JsonLike + Contain`. (Note: `@>`/`<@` are removed only on
//! *scalar* encrypted columns; on JSON they are the primary query surface.)

use std::collections::HashMap;
use std::sync::OnceLock;

use eql_bindings::v3;
use eql_mapper::{DomainIdentity, EqlTrait, EqlTraits, TokenType};

/// `typname` (e.g. `eql_v3_integer_ord`) → capabilities, inverted once from the
/// `eql-bindings` catalog. Keyed only by the public column domains; the token
/// type is recovered from the typname via [`DomainIdentity::from_domain_name`].
fn catalog() -> &'static HashMap<String, EqlTraits> {
    static MAP: OnceLock<HashMap<String, EqlTraits>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::new();
        for domain in v3::all() {
            // `sql_domain()` is schema-qualified, e.g. `public.eql_v3_integer_ord`.
            let qualified = domain.sql_domain();
            let typname = qualified.rsplit('.').next().unwrap_or(qualified);

            // Only public column domains have a parseable token type; this skips
            // the `eql_v3.query_*` operand twins that `all()` also yields.
            if TokenType::from_domain_name(typname).is_some() {
                map.insert(
                    typname.to_string(),
                    traits_from_terms(domain.term_json_keys()),
                );
            }
        }
        map
    })
}

/// Resolve a Postgres domain typname to its inert v3 domain identity and
/// capabilities, or `None` if it is not a recognised v3 EQL domain.
pub(crate) fn resolve(typname: &str) -> Option<(DomainIdentity, EqlTraits)> {
    let traits = *catalog().get(typname)?;
    let identity = DomainIdentity::from_domain_name(typname)?;
    Some((identity, traits))
}

fn traits_from_terms(term_keys: Option<&[&str]>) -> EqlTraits {
    match term_keys {
        // JSON SteVec domains: `->`/`->>` (JsonLike) plus `@>`/`<@` containment
        // (Contain), per the installed v3 SQL. The per-entry terms (hm/op) are
        // not column-level capabilities.
        None => [EqlTrait::JsonLike, EqlTrait::Contain]
            .into_iter()
            .collect(),
        Some(terms) => terms
            .iter()
            .filter_map(|term| match *term {
                "hm" => Some(EqlTrait::Eq),
                "op" | "ob" => Some(EqlTrait::Ord),
                "bf" => Some(EqlTrait::TokenMatch),
                // `c` is the storage-only source-ciphertext term; anything else
                // is unknown and contributes no capability.
                _ => None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn traits(typname: &str) -> EqlTraits {
        resolve(typname)
            .unwrap_or_else(|| panic!("{typname} did not resolve"))
            .1
    }

    fn token(typname: &str) -> TokenType {
        resolve(typname).unwrap().0.token
    }

    #[test]
    fn storage_only_domain_has_no_capabilities() {
        assert_eq!(traits("eql_v3_integer"), EqlTraits::none());
        assert_eq!(traits("eql_v3_text"), EqlTraits::none());
        // boolean is storage-only by design (a two-value column leaks its
        // distribution under any index).
        assert_eq!(traits("eql_v3_boolean"), EqlTraits::none());
    }

    #[test]
    fn eq_domain_implements_eq_only() {
        assert_eq!(traits("eql_v3_integer_eq"), EqlTraits::from(EqlTrait::Eq));
    }

    #[test]
    fn ord_domains_imply_eq() {
        // `op` and `ob` both back Ord; Ord implies Eq.
        let ord = EqlTraits::from(EqlTrait::Ord);
        assert_eq!(traits("eql_v3_integer_ord"), ord); // op
        assert_eq!(traits("eql_v3_integer_ord_ope"), ord); // op
        assert_eq!(traits("eql_v3_integer_ord_ore"), ord); // ob
        assert!(traits("eql_v3_integer_ord").eq); // Ord ⇒ Eq
    }

    #[test]
    fn match_domain_implements_token_match() {
        assert_eq!(
            traits("eql_v3_text_match"),
            EqlTraits::from(EqlTrait::TokenMatch)
        );
    }

    #[test]
    fn text_ord_carries_the_hm_equality_exception() {
        // Lexicographic ORE/OPE over text is not equality-lossless, so text_ord*
        // stores `hm` alongside its ordering term — Eq is explicit, not merely
        // implied.
        let eq_ord: EqlTraits = [EqlTrait::Eq, EqlTrait::Ord].into_iter().collect();
        assert_eq!(traits("eql_v3_text_ord"), eq_ord); // [hm, op]
        assert_eq!(traits("eql_v3_text_ord_ore"), eq_ord); // [hm, ob]
    }

    #[test]
    fn search_domains_implement_eq_ord_and_match() {
        let all_three: EqlTraits = [EqlTrait::Eq, EqlTrait::Ord, EqlTrait::TokenMatch]
            .into_iter()
            .collect();
        assert_eq!(traits("eql_v3_text_search"), all_three); // [hm, op, bf]
        assert_eq!(traits("eql_v3_text_search_ore"), all_three); // [hm, ob, bf]
    }

    #[test]
    fn json_search_domain_is_json_like_and_contain() {
        // Verified against cipherstash-encrypt.sql: -> / ->> (JsonLike) and
        // @> / <@ (Contain) are real operators on eql_v3_json_search.
        let json_caps: EqlTraits = [EqlTrait::JsonLike, EqlTrait::Contain]
            .into_iter()
            .collect();
        assert_eq!(traits("eql_v3_json_search"), json_caps);
    }

    #[test]
    fn token_type_is_parsed_across_suffixes() {
        assert_eq!(token("eql_v3_integer_ord_ore"), TokenType::Integer);
        assert_eq!(token("eql_v3_bigint_eq"), TokenType::BigInt);
        assert_eq!(token("eql_v3_text_search"), TokenType::Text);
        assert_eq!(token("eql_v3_timestamp_ord"), TokenType::Timestamp);
        assert_eq!(token("eql_v3_json_search"), TokenType::Json);
    }

    #[test]
    fn domain_identity_carries_the_typname() {
        let (identity, _) = resolve("eql_v3_integer_ord").unwrap();
        assert_eq!(identity.domain.value, "eql_v3_integer_ord");
        assert_eq!(identity.token, TokenType::Integer);
    }

    #[test]
    fn non_eql_domain_names_do_not_resolve() {
        assert!(resolve("jsonb").is_none());
        assert!(resolve("text").is_none());
        assert!(resolve("eql_v2_encrypted").is_none());
        assert!(resolve("").is_none());
    }

    #[test]
    fn every_public_column_domain_resolves_to_a_known_token_type() {
        // Guards against a new token type being added upstream that this loader
        // silently drops. Only `public.eql_v3_*` column domains are user-facing
        // column types; the `eql_v3.query_*` operand twins that `all()` also
        // yields (e.g. `eql_v3.query_json`) are deliberately not resolved.
        let mut checked = 0;
        for domain in v3::all() {
            let qualified = domain.sql_domain();
            if let Some(typname) = qualified.strip_prefix("public.") {
                assert!(
                    resolve(typname).is_some(),
                    "public column domain {typname} did not resolve to a token type"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no public column domains found in the catalog");
    }

    #[test]
    fn query_operand_twins_are_not_resolved_as_column_domains() {
        // `all()` yields at least one `eql_v3.query_*` twin; those are operands,
        // never column types, so they must not resolve here.
        assert!(resolve("query_json").is_none());
        assert!(resolve("query_integer_eq").is_none());
    }
}
