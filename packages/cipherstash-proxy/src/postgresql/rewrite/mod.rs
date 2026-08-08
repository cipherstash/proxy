use bytes::BytesMut;

pub mod bind;
pub mod data_row;
pub mod param_description;
pub mod parse;
pub mod query;
pub mod row_description;

pub type Name = bytes::Bytes;
pub const NULL: i32 = -1;
pub const UNSPECIFIED_TYPE_OID: i32 = 0;

pub fn maybe_json(bytes: &BytesMut) -> bool {
    bytes.first() == Some(&b'{')
}

pub fn maybe_jsonb(bytes: &BytesMut) -> bool {
    bytes.len() > 3 && bytes[0] == 1 && bytes[1] == b'{'
}
