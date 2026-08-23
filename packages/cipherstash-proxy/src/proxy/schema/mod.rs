//! Schema loading, committed snapshots, and connection-local transactional overlays.

/// Resolves EQL domain identities and capabilities.
mod eql_domains;
/// Loads and atomically publishes authoritative committed snapshots.
mod manager;
/// Coordinates schema state with PostgreSQL transaction and protocol events.
mod middleware;

pub use manager::{CommittedSchemaStore, SchemaManager};
pub use middleware::SchemaMiddleware;
