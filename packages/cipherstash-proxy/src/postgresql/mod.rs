mod column_mapper;
mod context;
mod data;
mod diagnostics;
mod driver;
mod error_handler;
mod format_code;
mod inbound_eql;
mod middleware;
mod parser;
mod rewrite;

pub use context::column::Column;
pub use context::Context;
pub use context::KeysetIdentifier;
pub use driver::handler;
pub(crate) use rewrite::Name;

/// Proxy-owned identity for correlating PostgreSQL protocol operations.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId(OperationIdInner);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum OperationIdInner {
    Protocol(pg_proto::OperationId),
    #[cfg(test)]
    Test(u64),
}

impl From<pg_proto::OperationId> for OperationId {
    fn from(id: pg_proto::OperationId) -> Self {
        Self(OperationIdInner::Protocol(id))
    }
}

#[cfg(test)]
pub(crate) fn test_operation_id() -> OperationId {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    OperationId(OperationIdInner::Test(
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_operation_ids_are_distinct() {
        assert_ne!(super::test_operation_id(), super::test_operation_id());
    }
}
