//! End-to-end correctness and soak workloads for CipherStash Proxy.
//!
//! The fixtures deliberately use EQL domains on uniquely named tables in
//! `public`, and workload SQL deliberately leaves those table names
//! unqualified. Proxy only loads schemas on its search path and EQL Mapper
//! resolves a flat table namespace; changing either invariant can silently
//! turn this into a passthrough workload that never exercises encryption.

pub mod conformance;
pub mod database;
pub mod resource;
pub mod soak;

pub const SCHEMA_MIGRATION: &str = include_str!("../migrations/0001_schema.sql");
pub const SEED_MIGRATION: &str = include_str!("../migrations/0002_seed.sql");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_are_public_and_include_encrypted_domains() {
        assert!(SCHEMA_MIGRATION.contains("CREATE TABLE public.burnin_"));
        assert!(SCHEMA_MIGRATION.contains("eql_v3_integer_ord"));
        assert!(SCHEMA_MIGRATION.contains("eql_v3_text"));
        assert!(SCHEMA_MIGRATION.contains("eql_v3_json"));
    }

    #[test]
    fn workload_queries_do_not_use_schema_qualified_fixture_names() {
        let workload = concat!(include_str!("conformance.rs"), include_str!("soak.rs"));
        assert!(!workload.contains("burnin_type_lab."));
        assert!(!workload.contains("burnin_commerce."));
    }
}
