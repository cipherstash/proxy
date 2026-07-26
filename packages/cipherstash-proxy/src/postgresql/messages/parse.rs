use super::{FrontendCode, Name, UNSPECIFIED_TYPE_OID};
use crate::{
    error::{Error, ProtocolError},
    postgresql::{context::statement::OutputParam, protocol::BytesMutReadString},
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

    /// Rewrites the declared param types to describe the params of the
    /// *rewritten* statement.
    ///
    /// EQL v3 encrypted columns are JSONB-backed domain types (e.g.
    /// `eql_v3_text_search`). JSONB is declared rather than the domain itself to
    /// avoid loading each domain's OID — PostgreSQL coerces JSONB to the domain
    /// if it passes the CHECK constraint.
    ///
    /// The client declares types for the params it wrote; the rewrite may have
    /// dropped or fused some of those, so each declaration is carried across to
    /// the output param that consumes it. An output param that carries an
    /// encrypted value is declared JSONB regardless — that is the wire type of
    /// every EQL payload, whatever the client thought it was binding.
    ///
    /// A client that declares no types at all (the common case — it lets the
    /// server infer them) is left alone: every output param is referenced by the
    /// rewritten SQL, so PostgreSQL can always infer them.
    pub fn rewrite_param_types(&mut self, output_params: &[OutputParam]) {
        if self.param_types.is_empty() {
            return;
        }

        let param_types = output_params
            .iter()
            .map(|output| match &output.column {
                Some(_) => Type::JSONB.oid() as i32,
                None => self
                    .param_types
                    .get(output.source.primary_input())
                    .copied()
                    .unwrap_or(UNSPECIFIED_TYPE_OID),
            })
            .collect::<Vec<_>>();

        if param_types != self.param_types {
            self.num_params = param_types.len() as i16;
            self.param_types = param_types;
            self.dirty = true;
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
        postgresql::{
            context::statement::{OutputParam, OutputParamSource},
            messages::parse::Parse,
            Column,
        },
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
        let output_params = vec![
            OutputParam {
                column: None,
                source: OutputParamSource::Input(0),
            },
            OutputParam {
                column: Some(column),
                source: OutputParamSource::Input(1),
            },
        ];

        parse.rewrite_param_types(&output_params);
        assert!(parse.requires_rewrite());
        assert_eq!(
            parse.param_types,
            vec![
                postgres_types::Type::INT2.oid() as i32,
                postgres_types::Type::JSONB.oid() as i32
            ]
        );
    }

    /// A rewrite that fuses two params into one must leave the client's
    /// declaration for the surviving param, not the one it happened to sit at.
    #[test]
    pub fn test_parse_rewrite_param_types_after_fusion() {
        log::init(LogConfig::default());
        let bytes = to_message(
             b"P\0\0\0J\0INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)\0\0\x02\0\0\0\x15\0\0\0\x15"
        );

        let mut parse = Parse::try_from(&bytes).unwrap();

        // Two input params collapse to a single native output param sourced
        // from input 1.
        let output_params = vec![OutputParam {
            column: None,
            source: OutputParamSource::Input(1),
        }];

        parse.rewrite_param_types(&output_params);
        assert!(parse.requires_rewrite());
        assert_eq!(parse.num_params, 1);
        assert_eq!(
            parse.param_types,
            vec![postgres_types::Type::INT2.oid() as i32]
        );
    }
}
