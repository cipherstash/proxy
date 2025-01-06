use super::context::{self, Context};
use super::format_code::FormatCode;
use super::messages::bind::{Bind, BindParam};
use super::messages::describe::Describe;
use super::messages::parse::Parse;
use super::messages::FrontendCode as Code;
use super::protocol::{self};
use crate::encrypt::Encrypt;
use crate::eql::Identifier;
use crate::error::{Error, MappingError};
use crate::log::MAPPER;
use crate::postgresql::context::Column;
use crate::postgresql::messages::execute::Execute;
use crate::postgresql::messages::query::Query;
use bytes::BytesMut;
use cipherstash_client::encryption::Plaintext;
use cipherstash_config::ColumnType;
use eql_mapper::{self, EqlMapperError, EqlValue, NativeValue, TableColumn};
use pg_escape::quote_literal;
use postgres_types::{FromSql, Type};
use sqlparser::ast::{CastKind, DataType, Expr, Value};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, info, warn};

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

pub struct Frontend<C, S>
where
    C: AsyncRead + Unpin,
    S: AsyncWrite + Unpin,
{
    client: C,
    server: S,
    encrypt: Encrypt,
    context: Context,
}

impl<C, S> Frontend<C, S>
where
    C: AsyncRead + Unpin,
    S: AsyncWrite + Unpin,
{
    pub fn new(client: C, server: S, encrypt: Encrypt, context: Context) -> Self {
        Frontend {
            client,
            server,
            encrypt,
            context,
        }
    }

    pub async fn rewrite(&mut self) -> Result<(), Error> {
        let bytes = self.read().await?;
        self.write(bytes).await?;
        Ok(())
    }

    pub async fn write(&mut self, bytes: BytesMut) -> Result<(), Error> {
        self.server.write_all(&bytes).await?;
        Ok(())
    }

    pub async fn read(&mut self) -> Result<BytesMut, Error> {
        let connection_timeout = self.encrypt.config.database.connection_timeout();
        let (code, mut bytes) =
            protocol::read_message_with_timeout(&mut self.client, connection_timeout).await?;

        match code.into() {
            Code::Query => {}
            Code::Describe => {
                self.describe_handler(&bytes).await?;
            }
            Code::Execute => {
                self.execute_handler(&bytes).await?;
            }
            Code::Parse => {
                match self.parse_handler(&bytes).await {
                    Ok(Some(b)) => bytes = b,
                    Ok(None) => (),
                    Err(e) => {
                        debug!("error parsing query: {}", e);
                        // This *should* be sufficient for escaping error messages as we're only
                        // using the string literal, and not identifiers
                        let quoted_error = quote_literal(format!("[CipherStash] {}", e).as_str());
                        let content =
                            format!("DO $$ begin raise exception {quoted_error}; END; $$;");
                        let query = Query { statement: content };
                        bytes = BytesMut::try_from(query)?;
                        debug!(
                            "frontend sending an exception-raising message: {:?}",
                            &bytes
                        );
                        // TODO: should some errors be bubbled up with `Err(e)?`
                    }
                }
            }
            Code::Bind => {
                if let Some(b) = self.bind_handler(&bytes).await? {
                    bytes = b;
                }
            }
            _code => {}
        }

        Ok(bytes)
    }

    async fn describe_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let describe = Describe::try_from(bytes)?;
        info!(target = MAPPER, "Describe {:?}", describe);
        self.context.describe(describe);
        Ok(())
    }

    async fn execute_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let execute = Execute::try_from(bytes)?;
        info!(target = MAPPER, "Execute {:?}", execute);
        self.context.execute(execute);
        Ok(())
    }

    async fn parse_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        if self.encrypt.config.disable_mapping() {
            return Ok(None);
        }

        let parse = Parse::try_from(bytes)?;

        let statement = Parser::new(&DIALECT)
            .try_with_sql(&parse.statement)?
            .parse_statement()?;

        warn!("cfg {:?}", self.encrypt.encrypt_config);

        if eql_mapper::requires_type_check(&statement) {
            let typed_statement = eql_mapper::type_check(self.encrypt.schema.load(), &statement)?;

            let param_columns = typed_statement
                .params
                .iter()
                .map(|param| match param {
                    eql_mapper::Value::Eql(EqlValue(TableColumn { table, column }))
                    | eql_mapper::Value::Native(NativeValue(Some(TableColumn { table, column }))) =>
                    {
                        let identifier = Identifier::from((table, column));

                        debug!(target = MAPPER, "Identifier {:?}", identifier);

                        match self.encrypt.get_column_config(&identifier) {
                            Some(config) => {
                                debug!(target = MAPPER, "Configured param {:?}", identifier);
                                let oid = column_type_to_oid(&config.cast_type);

                                let col = Column {
                                    identifier,
                                    config,
                                    postgres_type: oid,
                                };

                                Some(col)
                            }
                            None => None,
                        }
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();

            let projection_columns = match typed_statement.projection {
                Some(projection) => match projection {
                    eql_mapper::Projection::WithColumns(columns) => columns
                        .iter()
                        .map(|col| match col {
                            eql_mapper::ProjectionColumn { ty, .. } => match ty {
                                eql_mapper::Value::Eql(EqlValue(TableColumn { table, column }))
                                | eql_mapper::Value::Native(NativeValue(Some(TableColumn {
                                    table,
                                    column,
                                }))) => {
                                    let identifier: Identifier = Identifier::from((table, column));

                                    match self.encrypt.get_column_config(&identifier) {
                                        Some(config) => {
                                            debug!(
                                                target = MAPPER,
                                                "Configured projection {:?}", identifier
                                            );
                                            let oid = column_type_to_oid(&config.cast_type);
                                            let col = Column {
                                                identifier,
                                                config,
                                                postgres_type: oid,
                                            };

                                            Some(col)
                                        }
                                        None => None,
                                    }
                                }
                                _ => None,
                            },
                        })
                        .collect::<Vec<_>>(),
                    eql_mapper::Projection::Empty => vec![],
                },
                None => vec![],
            };

            debug!(
                target = MAPPER,
                "Statment added to context: {:?}", parse.name
            );

            self.context.add(
                parse.name.to_owned(),
                context::Statement::mapped(
                    typed_statement.statement.clone(),
                    parse.param_types.clone(),
                    param_columns.clone(),
                    projection_columns.clone(),
                ),
            );
        }

        if parse.should_rewrite() {
            let bytes = BytesMut::try_from(parse)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    ///
    /// Handle Bind messages
    ///
    /// Flow
    ///
    ///     Fetch the statement from the context
    ///     Fetch the statement param types
    ///         Only configured params have Some(param_type)
    ///
    ///     For each bind param
    ///         If Some(param_type) exists
    ///             Decode the parameter into the correct native type
    ///             Encrypt the param
    ///             Update the bind param with the encrypted value
    ///
    ///
    async fn bind_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        let mut bind = Bind::try_from(bytes)?;
        warn!("BIND ==============================");
        warn!("bind.portal {:?}", &bind.portal);
        warn!("bind.prepared_statement {:?}", &bind.prepared_statement);

        if let Some(statement) = self.context.get(&bind.prepared_statement) {
            // if let Some(param_cols) = self.context.get_param_columns(&bind.prepared_statement) {
            let param_columns = statement.param_columns.clone();
            let plaintexts = bind
                .param_values
                .iter_mut()
                .zip(param_columns.iter())
                .map(|(param, col)| match col {
                    Some(col) => {
                        debug!(target = MAPPER, "Mapping param: {col:?}");
                        to_plaintext(&param, &col)
                    }
                    None => Ok(None),
                })
                .collect::<Result<Vec<_>, _>>()?;

            debug!(target = MAPPER, "Mapping params: {plaintexts:?}");
            let encrypted = self.encrypt.encrypt(plaintexts, param_columns).await?;

            warn!("//////////////////////////////////");
            debug!(target = MAPPER, "Encrypted: {encrypted:?}");

            bind.update_from_ciphertext(encrypted)?;

            self.context.add_portal(
                bind.portal.to_owned(),
                context::Portal::new(statement.clone(), bind.result_columns_format_codes.clone()),
            );
        }

        warn!("/BIND ==============================");

        if bind.should_rewrite() {
            let bytes = BytesMut::try_from(bind)?;
            debug!(target = MAPPER, "Mapped params {bytes:?}");
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }
}

fn column_type_to_oid(col_type: &ColumnType) -> postgres_types::Type {
    match col_type {
        ColumnType::Boolean => postgres_types::Type::BOOL,
        ColumnType::BigInt => postgres_types::Type::INT8,
        ColumnType::BigUInt => postgres_types::Type::INT8,
        ColumnType::Date => postgres_types::Type::DATE,
        ColumnType::Decimal => postgres_types::Type::NUMERIC,
        ColumnType::Float => postgres_types::Type::FLOAT8,
        ColumnType::Int => postgres_types::Type::INT4,
        ColumnType::SmallInt => postgres_types::Type::INT2,
        ColumnType::Timestamp => postgres_types::Type::TIMESTAMPTZ,
        ColumnType::Utf8Str => postgres_types::Type::TEXT,
        ColumnType::JsonB => postgres_types::Type::JSONB,
    }
}

fn to_plaintext(param: &BindParam, col: &Column) -> Result<Option<Plaintext>, Error> {
    if param.is_null() {
        return Ok(None);
    }
    let pt = match param.format_code {
        FormatCode::Text => text_to_plaintext(param, col)?,
        FormatCode::Binary => binary_to_plaintext(param, col)?,
    };
    Ok(Some(pt))
}

fn text_to_plaintext(param: &BindParam, col: &Column) -> Result<Plaintext, Error> {
    let as_str = param.as_string()?;
    let ty = &col.postgres_type;
    match ty {
        &Type::BOOL => {
            let val = match as_str.as_str() {
                "TRUE" | "true" | "t" | "y" | "yes" | "on" | "1" => true,
                "FALSE" | "f" | "false" | "n" | "no" | "off" | "0" => false,
                _ => Err(MappingError::InvalidParameter {
                    name: ty.name().to_owned(),
                    oid: ty.oid() as i32,
                })?,
            };
            Ok(Plaintext::Boolean(Some(val)))
        }
        &Type::DATE => {
            unimplemented!("DATE")
        }
        &Type::NUMERIC => {
            unimplemented!("NUMERIC")
        }
        &Type::FLOAT8 => {
            let val = as_str
                .parse()
                .map_err(|_e| MappingError::InvalidParameter {
                    name: ty.name().to_owned(),
                    oid: ty.oid() as i32,
                })?;
            Ok(Plaintext::Float(Some(val)))
        }
        &Type::INT8 => {
            let val = as_str
                .parse()
                .map_err(|_e| MappingError::InvalidParameter {
                    name: ty.name().to_owned(),
                    oid: ty.oid() as i32,
                })?;
            Ok(Plaintext::BigInt(Some(val)))
        }
        &Type::INT4 => {
            let val = as_str
                .parse()
                .map_err(|_e| MappingError::InvalidParameter {
                    name: ty.name().to_owned(),
                    oid: ty.oid() as i32,
                })?;
            Ok(Plaintext::Int(Some(val)))
        }
        &Type::INT2 => {
            let val = as_str
                .parse()
                .map_err(|_e| MappingError::InvalidParameter {
                    name: ty.name().to_owned(),
                    oid: ty.oid() as i32,
                })?;
            Ok(Plaintext::SmallInt(Some(val)))
        }
        &Type::TIMESTAMPTZ => {
            unimplemented!("TIMESTAMPTZ")
        }
        &Type::TEXT => {
            let val = as_str.to_owned();
            Ok(Plaintext::Utf8Str(Some(val)))
        }
        &Type::JSONB => {
            unimplemented!("JSONB")
        }
        ty => Err(MappingError::UnsupportedParameterType {
            name: ty.name().to_owned(),
            oid: ty.oid() as i32,
        }
        .into()),
    }
}

fn binary_to_plaintext(param: &BindParam, col: &Column) -> Result<Plaintext, Error> {
    match &col.postgres_type {
        &Type::BOOL => {
            let val = <bool>::from_sql(&Type::BOOL, &param.bytes).unwrap();
            Ok(Plaintext::Boolean(Some(val)))
        }
        &Type::DATE => {
            unimplemented!("DATE")
        }
        &Type::NUMERIC => {
            unimplemented!("NUMERIC")
        }
        &Type::FLOAT8 => {
            // let bytes = param.bytes.to_vec();
            // let val = f64::from_be_bytes(bytes.try_into().map_err(|_| Error::Unknown)?);
            let val = <f64>::from_sql(&Type::FLOAT8, &param.bytes).unwrap();
            Ok(Plaintext::Float(Some(val)))
        }
        &Type::INT8 => {
            let val = <i64>::from_sql(&Type::INT8, &param.bytes).unwrap();
            Ok(Plaintext::BigInt(Some(val)))
        }
        &Type::INT4 => {
            // let bytes = param.bytes.to_vec();
            // let val = i32::from_be_bytes(bytes.try_into().map_err(|_| Error::Unknown)?);
            let val = <i32>::from_sql(&Type::INT4, &param.bytes).unwrap();
            Ok(Plaintext::Int(Some(val)))
        }
        &Type::INT2 => {
            // let bytes = param.bytes.to_vec();
            // let val = i16::from_be_bytes(bytes.try_into().map_err(|_| Error::Unknown)?);
            let val = <i16>::from_sql(&Type::INT2, &param.bytes).unwrap();
            Ok(Plaintext::SmallInt(Some(val)))
        }
        &Type::TIMESTAMPTZ => {
            unimplemented!("TIMESTAMPTZ")
        }
        &Type::TEXT => {
            // let val = param.read_string()?;
            let val = <String>::from_sql(&Type::TEXT, &param.bytes).unwrap();
            Ok(Plaintext::Utf8Str(Some(val)))
        }
        &Type::JSONB => {
            // let val = param.read_string()?;
            // Ok(Plaintext::Utf8Str(Some(val)))
            unimplemented!("JSONB")
        }
        ty => Err(MappingError::UnsupportedParameterType {
            name: ty.name().to_owned(),
            oid: ty.oid() as i32,
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {

    use std::ffi::CString;

    use crate::{
        log,
        postgresql::{
            format_code::FormatCode, frontend::to_plaintext, messages::bind::BindParam, Column,
        },
        Identifier,
    };
    use bytes::{BufMut, BytesMut};
    use cipherstash_client::encryption::Plaintext;
    use cipherstash_config::{ColumnConfig, ColumnMode, ColumnType};
    use postgres_types::{FromSql, Type};
    use tracing::info;

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    fn column(ty: Type) -> Column {
        Column {
            identifier: Identifier::new("table", "column"),
            config: ColumnConfig {
                name: "column".to_owned(),
                in_place: false,
                cast_type: ColumnType::Utf8Str,
                indexes: vec![],
                mode: ColumnMode::PlaintextDuplicate,
            },
            postgres_type: ty,
        }
    }
    #[test]
    pub fn bind_param_to_plaintext_i64() {
        log::init();

        // Binary
        let val: i64 = 42;
        let mut bytes = BytesMut::with_capacity(8);
        bytes.put_i64(val);
        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = to_plaintext(&param, &column(Type::INT8)).unwrap().unwrap();
        assert_eq!(pt, Plaintext::BigInt(Some(val)));

        // Text
        let val: i64 = 42;
        let s = CString::new("42").unwrap();
        let bytes = s.as_bytes_with_nul();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = to_plaintext(&param, &column(Type::INT8)).unwrap().unwrap();
        assert_eq!(pt, Plaintext::BigInt(Some(val)));
    }

    #[test]
    pub fn bind_param_to_plaintext_boolean() {
        log::init();

        // Binary
        let val = true;
        let mut bytes = BytesMut::with_capacity(1);
        bytes.put_u8(true as u8);
        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = to_plaintext(&param, &column(Type::BOOL)).unwrap().unwrap();
        assert_eq!(pt, Plaintext::Boolean(Some(val)));

        // Text
        let val = true;

        let s = CString::new("true").unwrap();
        let bytes = s.as_bytes_with_nul();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = to_plaintext(&param, &column(Type::BOOL)).unwrap().unwrap();
        assert_eq!(pt, Plaintext::Boolean(Some(val)));
    }

    #[test]
    pub fn bind_param_to_plaintext_timestamp() {
        log::init();

        let val: i64 = 42;

        let mut bytes = BytesMut::with_capacity(8);
        bytes.put_i64(val);
        let r = <i64>::from_sql(&Type::INT8, &bytes).unwrap();

        info!("{:?}", r);

        // let param = BindParam::new(FormatCode::Binary, bytes);

        // let b = bytes.to_vec();

        // let pt = to_plaintext(&param, postgres_types::Type::INT8).unwrap();
        // assert_eq!(pt, Plaintext::BigInt(Some(val)));
    }
}
