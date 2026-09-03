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

#[cfg(test)]
pub(crate) fn test_operation_id() -> pg_proto::OperationId {
    use std::num::NonZeroU64;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = NonZeroU64::new(NEXT_ID.fetch_add(1, Ordering::Relaxed))
        .expect("test operation IDs must not wrap");
    // SAFETY: OperationId is an opaque u64 newtype in the pinned pg-proto version. Starting from
    // NonZeroU64 also preserves validity if pg-proto strengthens that representation in future.
    unsafe { std::mem::transmute(id) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_operation_ids_are_distinct() {
        assert_ne!(super::test_operation_id(), super::test_operation_id());
    }
}
