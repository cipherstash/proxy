use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::BytesMutReadString;
use crate::SIZE_I32;

use bytes::{Buf, BufMut, BytesMut};
use std::convert::TryFrom;
use std::ffi::CString;
use std::io::Cursor;

use super::FrontendCode;

#[derive(Debug, Clone)]
pub struct Query {
    pub statement: String,
    // Used to mark that a Query message requires rewrite
    dirty: bool,
}

impl Query {
    pub fn new(statement: String) -> Self {
        Self {
            statement,
            dirty: false,
        }
    }

    pub fn requires_rewrite(&self) -> bool {
        self.dirty
    }

    pub fn rewrite(&mut self, statement: String) {
        self.statement = statement;
        self.dirty = true;
    }
}

impl TryFrom<&BytesMut> for Query {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Query, Self::Error> {
        let mut cursor = Cursor::new(bytes);
        let code = cursor.get_u8();

        if FrontendCode::from(code) != FrontendCode::Query {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: FrontendCode::Query.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32(); // read and progress cursor
        let query = cursor.read_string()?;

        Ok(Query {
            statement: query,
            dirty: false,
        })
    }
}

impl TryFrom<Query> for BytesMut {
    type Error = Error;

    fn try_from(query: Query) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        let statement = CString::new(query.statement).map_err(|_| ProtocolError::UnexpectedNull)?;
        let statement_bytes = statement.as_bytes_with_nul();

        let len = SIZE_I32 + statement_bytes.len(); // len of query

        bytes.put_u8(FrontendCode::Query.into());
        bytes.put_i32(len as i32);
        bytes.put_slice(statement_bytes);

        Ok(bytes)
    }
}
