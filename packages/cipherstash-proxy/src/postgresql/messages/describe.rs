use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::BytesMutReadString;
use crate::{SIZE_I32, SIZE_U8};

use bytes::{Buf, BufMut, BytesMut};
use std::convert::TryFrom;
use std::ffi::CString;
use std::io::Cursor;

use super::{FrontendCode, Name};

///
/// Describe b'D' (Frontend) message.
///
/// See: <https://www.postgresql.org/docs/current/protocol-message-formats.html>
///
///     Byte1('D')
///     Identifies the message as a Describe command.
///
///     Int32
///     Length of message contents in bytes, including self.
///
///     Byte1
///     'S' to describe a prepared statement; or 'P' to describe a portal.
///
///     String
///     The name of the prepared statement or portal to describe (an empty string selects the unnamed prepared statement or portal).

#[derive(Debug, Clone)]
pub(crate) struct Describe {
    pub target: Target,
    pub name: Name,
}

impl TryFrom<&BytesMut> for Describe {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Describe, Self::Error> {
        let mut cursor = Cursor::new(bytes);
        let code = cursor.get_u8();

        if FrontendCode::from(code) != FrontendCode::Describe {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: FrontendCode::Describe.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32(); // read and progress cursor
        let target = cursor.get_u8();
        let target = Target::try_from(target)?;
        let name = cursor.read_string()?;
        let name = Name::from(name);

        Ok(Describe { target, name })
    }
}

impl TryFrom<Describe> for BytesMut {
    type Error = Error;

    fn try_from(describe: Describe) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        let name = CString::new(describe.name.as_str())?;
        let name = name.as_bytes_with_nul();

        let len = SIZE_I32 + SIZE_U8 + name.len();

        bytes.put_u8(FrontendCode::Describe.into());
        bytes.put_i32(len as i32);
        bytes.put_u8(describe.target as u8);
        bytes.put_slice(name);

        Ok(bytes)
    }
}
