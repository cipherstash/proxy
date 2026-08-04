use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::{decode_frontend_frame, encode_frontend_message};

use bytes::{Bytes, BytesMut};
use pg_proto::codec::{Close as PgClose, DescribeTarget, FrontendMessage};
use std::convert::TryFrom;

use super::target::Target;
use super::Name;

/// Proxy state extracted from a typed frontend `Close` message.
#[derive(Debug, Clone)]
pub(crate) struct Close {
    pub target: Target,
    pub name: Name,
}

impl TryFrom<&BytesMut> for Close {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Close, Self::Error> {
        let FrontendMessage::Close(close) = decode_frontend_frame(bytes)? else {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: 'C',
                received: bytes.first().copied().unwrap_or_default() as char,
            }
            .into());
        };
        let target = match close.target {
            DescribeTarget::Statement => Target::Statement,
            DescribeTarget::Portal => Target::Portal,
        };
        let name = Name::from(String::from_utf8_lossy(&close.name).into_owned());

        Ok(Close { target, name })
    }
}

impl TryFrom<Close> for BytesMut {
    type Error = Error;

    fn try_from(close: Close) -> Result<BytesMut, Error> {
        let target = match close.target {
            Target::Statement => DescribeTarget::Statement,
            Target::Portal => DescribeTarget::Portal,
        };
        encode_frontend_message(&FrontendMessage::Close(PgClose {
            target,
            name: Bytes::copy_from_slice(close.name.as_str().as_bytes()),
        }))
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
