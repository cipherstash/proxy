use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::BytesMutReadString;
use crate::{SIZE_I32, SIZE_U8};

use bytes::{Buf, BufMut, BytesMut};
use std::convert::TryFrom;
use std::ffi::CString;
use std::io::Cursor;

use super::{Destination, FrontendCode};

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
    pub name: Destination,
}

///
/// The target of the describe message.
///
/// Valid values are PreparedStatment or Portal
///
/// A Portal is a parsed statement PLUS any bound parameters
/// Describe with `Target::Portal` returns the RowDescription describing the result set.
/// The assuumption is that the parameters are already bound to the portal, so the Describe message is not required to include any parameter information.
///
/// Calls to Execute are made on a Portal (not a prepared statement) as execute requires any bound parameters
///
/// A PreparedStatement is the parsed statement
/// Describe with `Target::PreparedStatement` returns a ParameterDescription followed by the RowDescription.
///
///
/// See https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-EXT-QUERY
///
#[derive(Debug, Clone)]
pub enum Target {
    Portal,
    PreparedStatement,
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
        let name = Destination::new(name);

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

impl TryFrom<u8> for Target {
    type Error = Error;

    fn try_from(t: u8) -> Result<Target, Error> {
        match t as char {
            'S' => Ok(Target::PreparedStatement),
            'P' => Ok(Target::Portal),
            t => Err(ProtocolError::UnexpectedDescribeTarget { target: t }.into()),
        }
    }
}
