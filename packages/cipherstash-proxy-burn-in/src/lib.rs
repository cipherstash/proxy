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

    #[test]
    fn ci_starts_proxy_against_a_database_without_encrypted_columns() {
        let workflow = include_str!("../../../.github/workflows/test.yml");
        let burn_in_job = workflow.split("  burn-in:").nth(1).expect("burn-in CI job");

        assert!(burn_in_job.contains("mise run eql:download"));
        assert!(burn_in_job.contains("mise run postgres:eql:teardown"));
        assert!(!burn_in_job.contains("mise run postgres:setup"));
    }

    #[test]
    fn eql_teardown_stops_on_sql_errors() {
        let tasks = include_str!("../../../mise.toml");
        let teardown = tasks
            .split("[tasks.\"postgres:eql:teardown\"]")
            .nth(1)
            .expect("EQL teardown task");

        assert!(teardown.contains("ON_ERROR_STOP=1"));
    }
}
