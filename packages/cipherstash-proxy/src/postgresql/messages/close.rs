use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::BytesMutReadString;
use crate::{SIZE_I32, SIZE_U8};

use bytes::{Buf, BufMut, BytesMut};
use std::convert::TryFrom;
use std::ffi::CString;
use std::io::Cursor;

use super::target::Target;
use super::{FrontendCode, Name};

///
/// Close b'C' (Frontend) message.
///
/// See: <https://www.postgresql.org/docs/current/protocol-message-formats.html>
///
///     Byte1('C')
///     Identifies the message as a Close command.
///
///     Int32
///     Length of message contents in bytes, including self.
///
///     Byte1
///     'S' to close a prepared statement; or 'P' to close a portal.
///
///     String
///     The name of the prepared statement or portal to close (an empty string selects the unnamed prepared statement or portal).

#[derive(Debug, Clone)]
pub(crate) struct Close {
    pub target: Target,
    pub name: Name,
}

impl TryFrom<&BytesMut> for Close {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Close, Self::Error> {
        let mut cursor = Cursor::new(bytes);
        let code = cursor.get_u8();

        if FrontendCode::from(code) != FrontendCode::Close {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: FrontendCode::Close.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32(); // read and progress cursor
        let target = cursor.get_u8();
        let target = Target::try_from(target)?;
        let name = cursor.read_string()?;
        let name = Name::from(name);

        Ok(Close { target, name })
    }
}

impl TryFrom<Close> for BytesMut {
    type Error = Error;

    fn try_from(close: Close) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        let name = CString::new(close.name.as_str())?;
        let name = name.as_bytes_with_nul();

        let len = SIZE_I32 + SIZE_U8 + name.len();

        bytes.put_u8(FrontendCode::Close.into());
        bytes.put_i32(len as i32);
        bytes.put_u8(close.target.into());
        bytes.put_slice(name);

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::LogConfig, log, postgresql::messages::Name};
    use bytes::BytesMut;
    use std::convert::TryFrom;

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn test_close_statement() {
        log::init(LogConfig::default());

        // Close unnamed prepared statement: C\0\0\0\x06S\0
        let bytes = to_message(b"C\0\0\0\x06S\0");
        let close = Close::try_from(&bytes).unwrap();

        assert!(matches!(close.target, Target::Statement));
        assert!(close.name.is_unnamed());
    }

    #[test]
    pub fn test_close_portal() {
        log::init(LogConfig::default());

        // Close unnamed portal: C\0\0\0\x06P\0
        let bytes = to_message(b"C\0\0\0\x06P\0");
        let close = Close::try_from(&bytes).unwrap();

        assert!(matches!(close.target, Target::Portal));
        assert!(close.name.is_unnamed());
    }

    #[test]
    pub fn test_close_named_statement() {
        log::init(LogConfig::default());

        // Close named prepared statement "stmt1": C\0\0\0\x0bSstmt1\0
        let bytes = to_message(b"C\0\0\0\x0bSstmt1\0");
        let close = Close::try_from(&bytes).unwrap();

        assert!(matches!(close.target, Target::Statement));
        assert_eq!(close.name.as_str(), "stmt1");
        assert!(!close.name.is_unnamed());
    }

    #[test]
    pub fn test_close_to_bytes() {
        log::init(LogConfig::default());

        let close = Close {
            target: Target::Portal,
            name: Name::from("portal1"),
        };

        let bytes = BytesMut::try_from(close).unwrap();
        let parsed = Close::try_from(&bytes).unwrap();

        assert!(matches!(parsed.target, Target::Portal));
        assert_eq!(parsed.name.as_str(), "portal1");
    }
}
