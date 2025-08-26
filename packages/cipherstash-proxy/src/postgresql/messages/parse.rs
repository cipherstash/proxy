use super::{FrontendCode, Name};
use crate::{
    error::{Error, ProtocolError},
    postgresql::{context::column::Column, protocol::BytesMutReadString},
    SIZE_I16, SIZE_I32,
};
use bytes::{Buf, BufMut, BytesMut};
use postgres_types::Type;
use std::{ffi::CString, io::Cursor};

#[derive(Debug, Clone)]
pub struct Parse {
    pub code: char,
    pub name: Name,
    pub statement: String,
    pub num_params: i16,
    pub param_types: Vec<i32>,
    dirty: bool,
}

impl Parse {
    pub fn requires_rewrite(&self) -> bool {
        self.dirty
    }

    ///
    /// Encrypted columns are the eql_v2_encrypted Domain Type
    /// eql_v2_encrypted wraps JSONB
    ///
    /// Using JSONB to avoid the complexity of loading the OID of eql_v2_encrypted
    /// PostgreSQL will coerce JSONB to eql_v2_encrypted if it passes the constaint check
    ///
    pub fn rewrite_param_types(&mut self, columns: &[Option<Column>]) {
        for (idx, col) in columns.iter().enumerate() {
            if self.param_types.get(idx).is_some() && col.is_some() {
                self.param_types[idx] = Type::JSONB.oid() as i32;
                self.dirty = true;
            }
        }
    }

    pub fn rewrite_statement(&mut self, statement: String) {
        self.statement = statement;
        self.dirty = true;
    }
}

impl TryFrom<&BytesMut> for Parse {
    type Error = Error;

    fn try_from(buf: &BytesMut) -> Result<Parse, Error> {
        let mut cursor = Cursor::new(buf);
        let code = cursor.get_u8() as char;

        if FrontendCode::from(code) != FrontendCode::Parse {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: FrontendCode::Parse.into(),
                received: code,
            }
            .into());
        }

        let _len = cursor.get_i32();
        let name = cursor.read_string()?;
        let name = Name::from(name);

        let statement = cursor.read_string()?;
        let num_params = cursor.get_i16();
        let mut param_types = Vec::new();

        for _ in 0..num_params {
            param_types.push(cursor.get_i32());
        }

        Ok(Parse {
            code,
            name,
            statement,
            num_params,
            param_types,
            dirty: false,
        })
    }
}

impl TryFrom<Parse> for BytesMut {
    type Error = Error;

    fn try_from(parse: Parse) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        let name = CString::new(parse.name.as_str())?;
        let name = name.as_bytes_with_nul();

        let statement = CString::new(parse.statement)?;
        let statement = statement.as_bytes_with_nul();

        let len = SIZE_I32 // len
                + name.len()
                + statement.len()
                + SIZE_I16 // num_params
                + SIZE_I32 * parse.param_types.len();

        bytes.put_u8(FrontendCode::Parse.into());
        bytes.put_i32(len as i32);
        bytes.put_slice(name);
        bytes.put_slice(statement);
        bytes.put_i16(parse.num_params);
        for param in parse.param_types {
            bytes.put_i32(param);
        }

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::LogConfig,
        log,
        postgresql::{messages::parse::Parse, Column},
        Identifier,
    };
    use bytes::BytesMut;
    use cipherstash_client::schema::{ColumnConfig, ColumnType};

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn test_parse() {
        log::init(LogConfig::default());
        let bytes = to_message(
             b"P\0\0\0J\0INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)\0\0\x02\0\0\0\x15\0\0\0\x15"
        );

        let expected = bytes.clone();

        let parse = Parse::try_from(&bytes).unwrap();

        let bytes = BytesMut::try_from(parse).unwrap();
        assert_eq!(bytes, expected);
    }

    #[test]
    pub fn test_parse_rewrite_param_types() {
        log::init(LogConfig::default());
        let bytes = to_message(
             b"P\0\0\0J\0INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)\0\0\x02\0\0\0\x15\0\0\0\x15"
        );

        let mut parse = Parse::try_from(&bytes).unwrap();

        let identifier = Identifier::new("table", "column");

        let config = ColumnConfig::build("column".to_string()).casts_as(ColumnType::SmallInt);

        let column = Column::new(identifier, config, None, eql_mapper::EqlTermVariant::Full);
        let columns = vec![None, Some(column)];

        parse.rewrite_param_types(&columns);
        assert!(parse.requires_rewrite());
    }
}
