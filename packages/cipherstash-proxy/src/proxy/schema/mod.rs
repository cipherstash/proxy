//! Schema loading, committed snapshots, and connection-local transactional overlays.

/// Resolves EQL domain identities and capabilities.
mod eql_domains;
/// Loads and atomically publishes authoritative committed snapshots.
mod manager;
/// Coordinates schema state with PostgreSQL transaction and protocol events.
mod middleware;

pub use manager::{CommittedSchemaStore, SchemaManager};
#[cfg(test)]
pub(crate) use middleware::committed_store_from_ddl;
pub use middleware::{SchemaMiddleware, TransactionStatus};

pub(crate) use middleware::postgres_identifier;
