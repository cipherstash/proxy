//! GIN index tests for JSONB containment operations
//!
//! Tests that the new EQL containment API enables GIN index usage:
//! - eql_v2.jsonb_array() returns jsonb[] with native hash support
//! - eql_v2.jsonb_contains() / jsonb_contained_by() helper functions
//!
//! Requires 500+ rows for PostgreSQL query planner to prefer GIN index over seq scan.

#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, insert, random_id, simple_query, trace, PG_LATEST, PROXY,
    };
    use serde_json::json;
    use tokio_postgres::SimpleQueryMessage;
    use tracing::info;

    const BULK_ROW_COUNT: usize = 500;
    const GIN_INDEX_NAME: &str = "encrypted_jsonb_gin_idx";
}
