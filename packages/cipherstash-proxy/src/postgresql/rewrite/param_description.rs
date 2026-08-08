//! CipherStash ParameterDescription rewriting.
use crate::log::MAPPER;
#[cfg(test)]
use crate::{
    error::{Error, ProtocolError},
    postgresql::test_codec::{decode_backend_frame, encode_backend_message},
};
#[cfg(test)]
use bytes::BytesMut;
use pg_proto::codec::BackendMessage;
use postgres_types::Type;
use tracing::debug;

///
/// Describe b't' (Backend) message.
///
/// See: <https://www.postgresql.org/docs/current/protocol-message-formats.html>
///
/// Byte1('t')
/// Identifies the message as a parameter description.
///
/// Int32
/// Length of message contents in bytes, including self.
///
/// Int16
/// The number of parameters used by the statement (can be zero).
///
/// For each parameter:
///     Int32
///     Specifies the object ID of the parameter data type.
///

#[derive(Debug)]
pub struct ParamDescription {
    pub types: Vec<i32>,
    dirty: bool,
}

impl ParamDescription {
    pub fn map_types(&mut self, mapped_types: &[Option<Type>]) {
        debug!(target: MAPPER, ?mapped_types);

        for (idx, t) in mapped_types.iter().enumerate() {
            if let Some(t) = t {
                self.types[idx] = t.oid() as i32;
                self.dirty = true;
            }
        }
    }

    /// Replaces the described params wholesale.
    ///
    /// PostgreSQL describes the params of the *rewritten* statement, but the
    /// client must be told about the params it wrote — a rewrite that fuses two
    /// params into one would otherwise describe too few, and the client would
    /// bind the wrong number of values.
    pub fn set_types(&mut self, types: Vec<i32>) {
        debug!(target: MAPPER, ?types);

        if types != self.types {
            self.types = types;
            self.dirty = true;
        }
    }

    pub fn requires_rewrite(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
impl TryFrom<&BytesMut> for ParamDescription {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<ParamDescription, Error> {
        let BackendMessage::ParameterDescription(types) = decode_backend_frame(bytes)? else {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: 't',
                received: bytes.first().copied().unwrap_or_default() as char,
            }
            .into());
        };

        Ok(ParamDescription {
            types: types.into_iter().map(|oid| oid as i32).collect(),
            dirty: false,
        })
    }
}

impl From<Vec<u32>> for ParamDescription {
    fn from(types: Vec<u32>) -> Self {
        Self {
            types: types.into_iter().map(|oid| oid as i32).collect(),
            dirty: false,
        }
    }
}

#[cfg(test)]
impl TryFrom<ParamDescription> for BytesMut {
    type Error = Error;

    fn try_from(parameter_description: ParamDescription) -> Result<BytesMut, Error> {
        let types = parameter_description
            .types
            .into_iter()
            .map(|oid| oid as u32)
            .collect();
        encode_backend_message(&BackendMessage::ParameterDescription(types))
    }
}

impl From<ParamDescription> for BackendMessage {
    fn from(parameter_description: ParamDescription) -> Self {
        Self::ParameterDescription(
            parameter_description
                .types
                .into_iter()
                .map(|oid| oid as u32)
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {

    use bytes::BytesMut;
    use tracing::info;

    use crate::{config::LogConfig, log};

    use super::ParamDescription;

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn map_parameter_types() {
        log::init(LogConfig::default());

        let mut pd = ParamDescription {
            types: vec![
                postgres_types::Type::TEXT.oid() as i32,
                postgres_types::Type::INT4.oid() as i32,
                postgres_types::Type::INT8.oid() as i32,
            ],
            dirty: false,
        };

        // No types to map, should not rewrite
        let mapped_types = vec![None, None, None];
        pd.map_types(&mapped_types);
        assert!(!pd.requires_rewrite());

        let mapped_types = vec![
            Some(postgres_types::Type::TEXT),
            None,
            Some(postgres_types::Type::TEXT),
        ];
        pd.map_types(&mapped_types);
        assert!(pd.requires_rewrite());

        let expected = vec![
            postgres_types::Type::TEXT.oid() as i32,
            postgres_types::Type::INT4.oid() as i32,
            postgres_types::Type::TEXT.oid() as i32,
        ];

        assert_eq!(pd.types, expected);
    }

    #[test]
    pub fn parse_parameter_description() {
        log::init(LogConfig::default());
        let bytes = to_message(b"t\0\0\0\x0e\0\x02\0\0\0\x14\0\0\x0e\xda");

        let expected = bytes.clone();

        let description = ParamDescription::try_from(&bytes).unwrap();

        info!("{:?}", description);

        assert_eq!(description.types.len(), 2);
        assert_eq!(
            description.types[0],
            postgres_types::Type::INT8.oid() as i32
        );
        assert_eq!(
            description.types[1],
            postgres_types::Type::JSONB.oid() as i32
        );

        let bytes = BytesMut::try_from(description).unwrap();
        assert_eq!(bytes, expected);
    }
}
