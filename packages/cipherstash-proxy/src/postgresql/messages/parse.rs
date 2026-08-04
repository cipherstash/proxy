use super::{Name, UNSPECIFIED_TYPE_OID};
use crate::postgresql::context::statement::OutputParam;
#[cfg(test)]
use crate::{
    error::{Error, ProtocolError},
    postgresql::test_codec::{decode_frontend_frame, encode_frontend_message},
};
use bytes::Bytes;
#[cfg(test)]
use bytes::BytesMut;
use eql_mapper::EqlTermVariant;
use pg_proto::codec::{FrontendMessage, Parse as PgParse};
use postgres_types::Type;

#[derive(Debug, Clone)]
pub struct Parse {
    pub name: Name,
    pub statement: String,
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
    /// A JSON *selector* is the exception. It is passed to the rewritten
    /// function as bare encrypted text — `eql_v3."->"(json, text)`,
    /// `eql_v3.jsonb_path_exists(json, text)` — not as a jsonb query payload, so
    /// declaring JSONB leaves PostgreSQL looking for an overload that does not
    /// exist:
    ///
    /// ```text
    /// ERROR: function eql_v3.jsonb_path_exists(eql_v3_json_search, jsonb) does not exist
    /// ```
    ///
    /// A client that declares no types at all (the common case — it lets the
    /// server infer them) is left alone: every output param is referenced by the
    /// rewritten SQL, so PostgreSQL can always infer them. That is why this only
    /// bites clients that send their own Parse OIDs, such as pgx in
    /// `cache_describe` mode.
    pub fn rewrite_param_types(&mut self, output_params: &[OutputParam]) {
        if self.param_types.is_empty() {
            return;
        }

        let param_types = output_params
            .iter()
            .map(|output| match &output.column {
                Some(column) => match column.eql_term {
                    EqlTermVariant::JsonAccessor | EqlTermVariant::JsonPath => {
                        Type::TEXT.oid() as i32
                    }
                    _ => Type::JSONB.oid() as i32,
                },
                None => self
                    .param_types
                    .get(output.source.primary_input())
                    .copied()
                    .unwrap_or(UNSPECIFIED_TYPE_OID),
            })
            .collect::<Vec<_>>();

        if param_types != self.param_types {
            self.param_types = param_types;
            self.dirty = true;
        }
    }

    pub fn rewrite_statement(&mut self, statement: String) {
        self.statement = statement;
        self.dirty = true;
    }
}

impl From<PgParse> for Parse {
    fn from(parse: PgParse) -> Self {
        Self {
            name: parse.statement,
            statement: String::from_utf8_lossy(&parse.query).into_owned(),
            param_types: parse
                .parameter_types
                .into_iter()
                .map(|oid| oid as i32)
                .collect(),
            dirty: false,
        }
    }
}

#[cfg(test)]
impl TryFrom<&BytesMut> for Parse {
    type Error = Error;

    fn try_from(buf: &BytesMut) -> Result<Parse, Error> {
        let FrontendMessage::Parse(parse) = decode_frontend_frame(buf)? else {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: 'P',
                received: buf.first().copied().unwrap_or_default() as char,
            }
            .into());
        };
        let name = parse.statement;
        let statement = String::from_utf8_lossy(&parse.query).into_owned();
        let param_types = parse
            .parameter_types
            .iter()
            .map(|oid| *oid as i32)
            .collect::<Vec<_>>();

        Ok(Parse {
            name,
            statement,
            param_types,
            dirty: false,
        })
    }
}

#[cfg(test)]
impl TryFrom<Parse> for BytesMut {
    type Error = Error;

    fn try_from(parse: Parse) -> Result<BytesMut, Error> {
        encode_frontend_message(&FrontendMessage::Parse(PgParse {
            statement: parse.name,
            query: Bytes::from(parse.statement),
            parameter_types: parse
                .param_types
                .into_iter()
                .map(|oid| oid as u32)
                .collect(),
        }))
    }
}

impl From<Parse> for FrontendMessage {
    fn from(parse: Parse) -> Self {
        Self::Parse(PgParse {
            statement: parse.name,
            query: Bytes::from(parse.statement),
            parameter_types: parse
                .param_types
                .into_iter()
                .map(|oid| oid as u32)
                .collect(),
        })
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
                query_operand: false,
            },
            OutputParam {
                column: Some(column),
                source: OutputParamSource::Input(1),
                query_operand: false,
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
            query_operand: false,
        }];

        parse.rewrite_param_types(&output_params);
        assert!(parse.requires_rewrite());
        assert_eq!(parse.param_types.len(), 1);
        assert_eq!(
            parse.param_types,
            vec![postgres_types::Type::INT2.oid() as i32]
        );
    }
}
