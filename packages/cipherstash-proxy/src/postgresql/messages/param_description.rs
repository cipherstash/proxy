use super::BackendCode;
use crate::{
    error::{Error, ProtocolError},
    log::MAPPER,
    SIZE_I16, SIZE_I32,
};
use bytes::{Buf, BufMut, BytesMut};
use postgres_types::Type;
use std::io::Cursor;
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

    pub fn requires_rewrite(&self) -> bool {
        self.dirty
    }
}

impl TryFrom<&BytesMut> for ParamDescription {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<ParamDescription, Error> {
        let mut cursor = Cursor::new(bytes);

        let code = cursor.get_u8();

        if BackendCode::from(code) != BackendCode::ParameterDescription {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: BackendCode::ParameterDescription.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32(); // move the cursor
        let count = cursor.get_i16() as usize;

        let mut types = vec![];
        for _idx in 0..count {
            let type_oid = cursor.get_i32();
            types.push(type_oid)
        }

        Ok(ParamDescription {
            types,
            dirty: false,
        })
    }
}

impl TryFrom<ParamDescription> for BytesMut {
    type Error = Error;

    fn try_from(parameter_description: ParamDescription) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        let count = parameter_description.types.len();
        let size_of_types = count * SIZE_I32;

        let len = SIZE_I32 + SIZE_I16 + size_of_types;

        bytes.put_u8(BackendCode::ParameterDescription.into());
        bytes.put_i32(len as i32);
        bytes.put_i16(count as i16);

        for type_oid in parameter_description.types.into_iter() {
            bytes.put_i32(type_oid);
        }

        Ok(bytes)
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
