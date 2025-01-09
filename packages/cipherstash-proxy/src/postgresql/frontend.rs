use std::str::FromStr;

use super::context::{self, Context};
use super::format_code::FormatCode;
use super::messages::bind::{Bind, BindParam};
use super::messages::describe::Describe;
use super::messages::parse::Parse;
use super::messages::FrontendCode as Code;
use super::protocol::{self};
use crate::encrypt::Encrypt;
use crate::eql::Identifier;
use crate::error::{EncryptError, Error, MappingError};
use crate::log::MAPPER;
use crate::postgresql::context::Column;
use crate::postgresql::messages::execute::Execute;
use crate::postgresql::messages::query::Query;
use bytes::BytesMut;
use chrono::NaiveDate;
use cipherstash_client::encryption::Plaintext;
use eql_mapper::{self, EqlValue, TableColumn};
use pg_escape::quote_literal;
use postgres_types::{FromSql, Type};
use rust_decimal::Decimal;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info};

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
        if self.encrypt.config.disable_mapping() {
            return Ok(());
        }

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
        let parse = Parse::try_from(bytes)?;

        let statement = Parser::new(&DIALECT)
            .try_with_sql(&parse.statement)?
            .parse_statement()?;

        if eql_mapper::requires_type_check(&statement) {
            let typed_statement = eql_mapper::type_check(self.encrypt.schema.load(), &statement)?;

            let param_columns = self.get_param_columns(&typed_statement)?;
            let projection_columns = self.get_projection_columns(&typed_statement)?;

            debug!(target = MAPPER, "Statement context: {:?}", parse.name);

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

    fn get_projection_columns(
        &mut self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let projection_columns = match &typed_statement.projection {
            Some(projection) => match projection {
                eql_mapper::Projection::WithColumns(columns) => columns
                    .iter()
                    .map(|col| {
                        let eql_mapper::ProjectionColumn { ty, .. } = col;
                        match ty {
                            eql_mapper::Value::Eql(EqlValue(TableColumn { table, column })) => {
                                let identifier: Identifier = Identifier::from((table, column));
                                debug!(target = MAPPER, "Encrypted column{:?}", identifier);
                                self.get_column(identifier)
                            }
                            _ => Ok(None),
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                eql_mapper::Projection::Empty => vec![],
            },
            None => vec![],
        };
        Ok(projection_columns)
    }

    fn get_param_columns(
        &mut self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let param_columns = typed_statement
            .params
            .iter()
            .map(|param| match param {
                eql_mapper::Value::Eql(EqlValue(TableColumn { table, column })) => {
                    let identifier = Identifier::from((table, column));
                    debug!(target = MAPPER, "Encrypted parameter {:?}", identifier);
                    self.get_column(identifier)
                }
                _ => Ok(None),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(param_columns)
    }

    fn get_column(&mut self, identifier: Identifier) -> Result<Option<Column>, Error> {
        match self.encrypt.get_column_config(&identifier) {
            Some(config) => {
                debug!(target = MAPPER, "Configured param {:?}", identifier);
                Ok(Some(Column::new(identifier, config)))
            }
            None => Err(EncryptError::UnknownColumn {
                table: identifier.table.to_owned(),
                column: identifier.column.to_owned(),
            }
            .into()),
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

        if let Some(statement) = self.context.get(&bind.prepared_statement) {
            let param_columns = statement.param_columns.clone();

            let plaintexts = bind
                .param_values
                .iter_mut()
                .zip(param_columns.iter())
                .map(|(param, col)| match col {
                    Some(col) => {
                        debug!(target = MAPPER, "Mapping param: {col:?}");
                        to_plaintext(param, col)
                    }
                    None => Ok(None),
                })
                .collect::<Result<Vec<_>, _>>()?;

            let encrypted = self.encrypt.encrypt(plaintexts, param_columns).await?;
            debug!(target = MAPPER, "Encrypted: {encrypted:?}");

            bind.rewrite(encrypted)?;

            self.context.add_portal(
                bind.portal.to_owned(),
                context::Portal::new(statement.clone(), bind.result_columns_format_codes.clone()),
            );
        }

        if bind.should_rewrite() {
            let bytes = BytesMut::try_from(bind)?;
            debug!(target = MAPPER, "Mapped params {bytes:?}");
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }
}

fn to_plaintext(param: &BindParam, col: &Column) -> Result<Option<Plaintext>, Error> {
    if param.is_null() {
        return Ok(None);
    }
    let pt = match param.format_code {
        FormatCode::Text => text_to_plaintext(param, col),
        FormatCode::Binary => binary_to_plaintext(param, col),
    }
    .map_err(|_| MappingError::InvalidParameter {
        table: col.identifier.table.to_owned(),
        column: col.identifier.table.to_owned(),
        postgres_type: col.postgres_type.name().to_owned(),
    })?;

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
                _ => Err(MappingError::CouldNotParseParameter)?,
            };
            Ok(Plaintext::Boolean(Some(val)))
        }
        &Type::DATE => {
            let val = NaiveDate::parse_from_str(&as_str, "%Y-%m-%d")?;
            Ok(Plaintext::NaiveDate(Some(val)))
        }
        &Type::FLOAT8 => {
            let val = as_str.parse()?;
            Ok(Plaintext::Float(Some(val)))
        }
        &Type::INT2 => {
            let val = as_str.parse()?;
            Ok(Plaintext::SmallInt(Some(val)))
        }
        &Type::INT4 => {
            let val = as_str.parse()?;
            Ok(Plaintext::Int(Some(val)))
        }
        &Type::INT8 => {
            let val = as_str.parse()?;
            Ok(Plaintext::BigInt(Some(val)))
        }
        &Type::NUMERIC => {
            let val = Decimal::from_str(&as_str)?;
            Ok(Plaintext::Decimal(Some(val)))
        }
        &Type::TEXT => {
            let val = as_str.to_owned();
            Ok(Plaintext::Utf8Str(Some(val)))
        }
        &Type::TIMESTAMPTZ => {
            unimplemented!("TIMESTAMPTZ")
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
            let val = <bool>::from_sql(&Type::BOOL, &param.bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Boolean(Some(val)))
        }
        &Type::DATE => {
            let val = <NaiveDate>::from_sql(&Type::DATE, &param.bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::NaiveDate(Some(val)))
        }
        &Type::FLOAT8 => {
            let val = <f64>::from_sql(&Type::FLOAT8, &param.bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Float(Some(val)))
        }
        &Type::INT2 => {
            let val = <i16>::from_sql(&Type::INT2, &param.bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::SmallInt(Some(val)))
        }
        &Type::INT4 => {
            let val = <i32>::from_sql(&Type::INT4, &param.bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Int(Some(val)))
        }
        &Type::INT8 => {
            let val = <i64>::from_sql(&Type::INT8, &param.bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::BigInt(Some(val)))
        }
        &Type::NUMERIC => {
            let val = <Decimal>::from_sql(&Type::NUMERIC, &param.bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Decimal(Some(val)))
        }
        &Type::TEXT => {
            let val = <String>::from_sql(&Type::TEXT, &param.bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Utf8Str(Some(val)))
        }
        &Type::TIMESTAMPTZ => {
            unimplemented!("TIMESTAMPTZ")
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

#[cfg(test)]
mod tests {

    use crate::{
        log,
        postgresql::{
            format_code::FormatCode, frontend::to_plaintext, messages::bind::BindParam, Column,
        },
        Identifier,
    };
    use bytes::{BufMut, BytesMut};
    use chrono::NaiveDate;
    use cipherstash_client::encryption::Plaintext;
    use cipherstash_config::{ColumnConfig, ColumnMode, ColumnType};
    use postgres_types::{FromSql, ToSql, Type};
    use std::ffi::CString;
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
    pub fn bind_param_to_plaintext_date() {
        log::init();

        // Binary
        let val = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();

        let mut bytes = BytesMut::new();
        val.to_sql_checked(&Type::DATE, &mut bytes);

        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = to_plaintext(&param, &column(Type::DATE)).unwrap().unwrap();
        assert_eq!(pt, Plaintext::NaiveDate(Some(val)));

        // Text
        let s = CString::new("2025-01-01").unwrap();
        let bytes = s.as_bytes_with_nul();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = to_plaintext(&param, &column(Type::DATE)).unwrap().unwrap();
        assert_eq!(pt, Plaintext::NaiveDate(Some(val)));
    }
}
