use bytes::BytesMut;

pub mod bind;
pub mod data_row;
pub mod error_response;
pub mod param_description;
pub mod parse;
pub mod query;
pub mod row_description;
pub type Name = bytes::Bytes;

pub const NULL: i32 = -1;

/// PostgreSQL's "unspecified type, infer it" param OID, used in `Parse` and
/// when a param's type is not known to the proxy.
pub const UNSPECIFIED_TYPE_OID: i32 = 0;

/// Returns whether a text value may contain a JSON object.
pub fn maybe_json(bytes: &BytesMut) -> bool {
    bytes.first() == Some(&b'{')
}

/// Returns whether a binary value may contain a JSONB object.
pub fn maybe_jsonb(bytes: &BytesMut) -> bool {
    bytes.len() > 3 && bytes[0] == 1 && bytes[1] == b'{'
}
