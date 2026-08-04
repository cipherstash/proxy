use super::Name;
use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::decode_frontend_frame;
use bytes::BytesMut;
use pg_proto::codec::FrontendMessage;
use std::convert::TryFrom;

#[derive(Debug, Clone)]
pub(crate) struct Execute {
    pub portal: Name,
    pub max_rows: i32,
}

impl TryFrom<&BytesMut> for Execute {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Execute, Self::Error> {
        let FrontendMessage::Execute(execute) = decode_frontend_frame(bytes)? else {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: 'E',
                received: bytes.first().copied().unwrap_or_default() as char,
            }
            .into());
        };
        let portal = Name::from(String::from_utf8_lossy(&execute.portal).into_owned());
        let max_rows = execute.max_rows;

        Ok(Execute { portal, max_rows })
    }
}
