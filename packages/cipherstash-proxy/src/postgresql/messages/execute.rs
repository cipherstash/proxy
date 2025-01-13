use super::{FrontendCode, Name};
use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::BytesMutReadString;
use bytes::{Buf, BytesMut};
use std::convert::TryFrom;
use std::io::Cursor;

#[derive(Debug, Clone)]
pub(crate) struct Execute {
    pub portal: Name,
    pub max_rows: i32,
}

impl TryFrom<&BytesMut> for Execute {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Execute, Self::Error> {
        let mut cursor = Cursor::new(bytes);
        let code = cursor.get_u8();

        if FrontendCode::from(code) != FrontendCode::Execute {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: FrontendCode::Execute.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32(); // read and progress cursor

        let portal = cursor.read_string()?;
        let portal = Name(portal);
        let max_rows = cursor.get_i32();

        Ok(Execute { portal, max_rows })
    }
}
