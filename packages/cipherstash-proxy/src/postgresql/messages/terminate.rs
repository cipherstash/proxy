use bytes::{BufMut, BytesMut};

use super::FrontendCode;

pub struct Terminate;

impl Terminate {
    pub fn message() -> BytesMut {
        let mut bytes = BytesMut::new();

        bytes.put_u8(FrontendCode::Terminate.into());
        bytes.put_i32(4);

        bytes
    }
}
