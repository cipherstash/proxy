use crate::{postgresql::messages::BackendCode, SIZE_I32, SIZE_U8};
use bytes::{BufMut, BytesMut};

/// Bind (Z) message.
/// See: <https://www.postgresql.org/docs/current/protocol-message-formats.html>
#[derive(Clone, Debug)]
pub struct ReadyForQuery;

impl From<ReadyForQuery> for BytesMut {
    fn from(_: ReadyForQuery) -> BytesMut {
        let mut bytes = BytesMut::new();

        let len = SIZE_I32 + SIZE_U8;

        bytes.put_u8(BackendCode::ReadyForQuery.into());
        bytes.put_i32(len as i32);
        bytes.put_u8(b'I');

        bytes
    }
}
