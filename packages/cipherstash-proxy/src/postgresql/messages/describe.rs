use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::{decode_frontend_frame, encode_frontend_message};

use bytes::{Bytes, BytesMut};
use pg_proto::codec::{Describe as PgDescribe, DescribeTarget, FrontendMessage};
use std::convert::TryFrom;

use super::target::Target;
use super::Name;

/// Proxy state extracted from a typed frontend `Describe` message.
#[derive(Debug, Clone)]
pub struct Describe {
    pub target: Target,
    pub name: Name,
}

impl TryFrom<&BytesMut> for Describe {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Describe, Self::Error> {
        let FrontendMessage::Describe(description) = decode_frontend_frame(bytes)? else {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: 'D',
                received: bytes.first().copied().unwrap_or_default() as char,
            }
            .into());
        };
        let target = match description.target {
            DescribeTarget::Statement => Target::Statement,
            DescribeTarget::Portal => Target::Portal,
        };
        let name = Name::from(String::from_utf8_lossy(&description.name).into_owned());

        Ok(Describe { target, name })
    }
}

impl TryFrom<Describe> for BytesMut {
    type Error = Error;

    fn try_from(describe: Describe) -> Result<BytesMut, Error> {
        let target = match describe.target {
            Target::Statement => DescribeTarget::Statement,
            Target::Portal => DescribeTarget::Portal,
        };
        encode_frontend_message(&FrontendMessage::Describe(PgDescribe {
            target,
            name: Bytes::copy_from_slice(describe.name.as_str().as_bytes()),
        }))
    }
}
