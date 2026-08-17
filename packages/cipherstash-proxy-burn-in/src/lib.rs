pub mod conformance;
pub mod database;
pub mod resource;
pub mod soak;

pub const SCHEMA_MIGRATION: &str = include_str!("../migrations/0001_schema.sql");
pub const SEED_MIGRATION: &str = include_str!("../migrations/0002_seed.sql");
