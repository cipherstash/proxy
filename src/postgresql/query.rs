use anyhow::bail;
use bytes::{Buf, BytesMut};
use std::convert::TryFrom;
use std::io::Cursor;

use super::{BytesMutReadString, QUERY};

#[derive(Debug, Clone)]
pub(crate) struct Query {
    pub statement: String,
}

impl TryFrom<&BytesMut> for Query {
    type Error = anyhow::Error;

    fn try_from(bytes: &BytesMut) -> Result<Query, Self::Error> {
        let mut cursor = Cursor::new(bytes);
        let code = cursor.get_u8();

        if code != QUERY {
            bail!("Invalid message code for Query {code}");
        }

        let _len = cursor.get_i32(); // read and progress cursor
        let query = cursor.read_string()?;

        Ok(Query { statement: query })
    }
}

// impl TryFrom<Query> for BytesMut {
//     type Error = Error;

//     fn try_from(query: Query) -> Result<BytesMut, Error> {
//         let mut bytes = BytesMut::new();

//         let query_str = CString::new(query.statement)?;
//         let query_slice = query_str.as_bytes_with_nul();

//         let len = size_of::<i32>() // len of len
//                          + query_slice.len(); // len of query

//         bytes.put_u8(FrontendCode::Query.into());
//         bytes.put_i32(len as i32);
//         bytes.put_slice(query_slice);

//         Ok(bytes)
//     }
// }
