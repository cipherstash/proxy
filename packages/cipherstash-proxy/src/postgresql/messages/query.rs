#[cfg(test)]
use crate::error::{Error, ProtocolError};
#[cfg(test)]
use crate::postgresql::protocol::{decode_frontend_frame, encode_frontend_message};

use bytes::Bytes;
#[cfg(test)]
use bytes::BytesMut;
use pg_proto::codec::FrontendMessage;
#[cfg(test)]
use std::convert::TryFrom;

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

impl From<Bytes> for Query {
    fn from(query: Bytes) -> Self {
        Self {
            statement: String::from_utf8_lossy(&query).into_owned(),
            dirty: false,
        }
    }
}

#[cfg(test)]
impl TryFrom<&BytesMut> for Query {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Query, Self::Error> {
        let FrontendMessage::Query(query) = decode_frontend_frame(bytes)? else {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: 'Q',
                received: bytes.first().copied().unwrap_or_default() as char,
            }
            .into());
        };

        Ok(Query {
            statement: String::from_utf8_lossy(&query).into_owned(),
            dirty: false,
        })
    }
}

#[cfg(test)]
impl TryFrom<Query> for BytesMut {
    type Error = Error;

    fn try_from(query: Query) -> Result<BytesMut, Error> {
        encode_frontend_message(&FrontendMessage::Query(Bytes::from(query.statement)))
    }
}

impl From<Query> for FrontendMessage {
    fn from(query: Query) -> Self {
        Self::Query(Bytes::from(query.statement))
    }
}
