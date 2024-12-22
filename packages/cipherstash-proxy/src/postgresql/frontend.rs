use std::collections::HashMap;

use super::context::{self, Context};
use super::messages::bind::Bind;
use super::messages::describe::Describe;
use super::messages::parse::Parse;
use super::messages::{describe, FrontendCode as Code};
use super::protocol::{self, Message};
use crate::encrypt::Encrypt;
use crate::eql;
use crate::eql::Identifier;
use crate::error::Error;
use crate::log::{DEVELOPMENT, MAPPER};
use crate::postgresql::messages::query::Query;
use bytes::BytesMut;
use cipherstash_config::ColumnType;
use eql_mapper::{self, EqlMapperError, EqlValue, NativeValue, TableColumn};
use pg_escape::quote_literal;
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
                if let Some(b) = self.describe_handler(&bytes).await? {
                    bytes = b;
                }
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

    async fn describe_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        let describe = Describe::try_from(bytes)?;
        info!("Describe{:?}", describe);
        self.context.describe(describe);
        Ok(None)
    }
    async fn parse_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        if self.encrypt.config.disable_mapping() {
            return Ok(None);
        }

        let parse = Parse::try_from(bytes)?;

        let statement = Parser::new(&DIALECT)
            .try_with_sql(&parse.statement)?
            .parse_statement()?;

        if eql_mapper::requires_type_check(&statement) {
            let typed_statement = eql_mapper::type_check(self.encrypt.schema.load(), &statement)?;

            let param_type_mapping = typed_statement
                .params
                .iter()
                .map(|param| match param {
                    eql_mapper::Value::Eql(EqlValue(TableColumn { table, column }))
                    | eql_mapper::Value::Native(NativeValue(Some(TableColumn { table, column }))) =>
                    {
                        let identifier = Identifier::from((table, column));

                        error!(target = MAPPER, "Identifier {:?}", identifier);

                        match self.encrypt.get_column_config(&identifier) {
                            Some(config) => {
                                debug!(target = MAPPER, "Configured parameter {:?}", identifier);
                                let oid = column_type_to_oid(&config.cast_type);
                                Some(oid)
                            }
                            None => None,
                        }
                    }
                    p => None,
                })
                .collect::<Vec<_>>();

            warn!("cfg {:?}", self.encrypt.encrypt_config);
            warn!("p {:?}", typed_statement.params);
            warn!("{:?}", param_type_mapping);

            // let plaintext_literals: Vec<eql::Plaintext> =
            //     convert_value_nodes_to_eql_plaintext(&typed_statement)?;

            // info!("==============================");
            // info!("{:?}", plaintext_literals);

            // let replacement_literal_values = self.encrypt_literals(plaintext_literals).await?;

            // info!("==============================");
            // info!("{:?}", replacement_literal_values);

            // let original_values_and_replacements =
            //     zip_with_original_value_ref(&typed_statement, replacement_literal_values);

            // info!("==============================");
            // info!("{:?}", original_values_and_replacements);

            // warn!("==============================");
            // warn!("==============================");

            // let transformed_statement =
            //     typed_statement.transform(original_values_and_replacements)?;

            // parse.statement = transformed_statement.to_string().clone();

            debug!(
                target = MAPPER,
                "Statment added to context: {:?}", parse.name
            );

            self.context.add(
                parse.name.to_owned(),
                context::Statement::mapped(
                    typed_statement.statement.clone(),
                    parse.param_types.clone(),
                    param_type_mapping.clone(),
                    typed_statement.params.clone(),
                    typed_statement.statement_type.clone(),
                ),
            );
        } else {
            self.context.add(
                parse.name.to_owned(),
                context::Statement::unmapped(statement, parse.param_types.clone()),
            );
        }

        if parse.should_rewrite() {
            let bytes = BytesMut::try_from(parse)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    async fn encrypt_literals(
        &mut self,
        plaintext_literals: Vec<eql::Plaintext>,
    ) -> Result<Vec<Expr>, Error> {
        let encrypted = self.encrypt.encrypt_mandatory(plaintext_literals).await?;

        Ok(encrypted
            .into_iter()
            .map(|ct| {
                serde_json::to_string(&ct).map(|ct| Expr::Cast {
                    kind: CastKind::DoubleColon,
                    expr: Box::new(Expr::Value(Value::SingleQuotedString(ct))),
                    data_type: DataType::JSONB,
                    format: None,
                })
            })
            .collect::<Result<Vec<_>, _>>()?)
    }

    async fn bind_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        let mut bind = Bind::try_from(bytes)?;
        warn!("BIND ==============================");
        warn!("{:?}", &bind.prepared_statement);
        if let Some(statement) = self.context.get(&bind.prepared_statement) {
            warn!("{:?}", statement);
            // bind.params.iter().zip()
            // let config = self.encrypt.column_config()
        }
        warn!("/BIND ==============================");
        let params = bind.to_plaintext()?;
        let encrypted = self.encrypt.encrypt(params).await?;

        bind.update_from_ciphertext(encrypted)?;

        if bind.should_rewrite() {
            let bytes = BytesMut::try_from(bind)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }
}

fn zip_with_original_value_ref<'ast>(
    typed_statement: &eql_mapper::TypedStatement<'ast>,
    encrypted_literals: Vec<Expr>,
) -> HashMap<&'ast Expr, Expr> {
    typed_statement
        .literals
        .iter()
        .map(|(_, original_node)| *original_node)
        .zip(encrypted_literals)
        .collect::<HashMap<_, _>>()
}

fn convert_value_nodes_to_eql_plaintext(
    typed_statement: &eql_mapper::TypedStatement<'_>,
) -> Result<Vec<eql::Plaintext>, EqlMapperError> {
    typed_statement
        .literals
        .iter()
        .map(|(EqlValue(TableColumn { table, column }), expr)| {
            if let Some(plaintext) = match expr {
                Expr::Value(Value::Number(number, _)) => Some(number.to_string()),
                Expr::Value(Value::SingleQuotedString(s)) => Some(s.to_owned()),
                Expr::Value(Value::Boolean(b)) => Some(b.to_string()),
                Expr::Value(Value::Null) => None,
                _ => None,
            } {
                Ok(eql::Plaintext {
                    identifier: Identifier::from((table, column)),
                    plaintext,
                    version: 1,
                    for_query: None,
                })
            } else {
                Err(EqlMapperError::UnsupportedValueVariant(expr.to_string()))
            }
        })
        .collect()
}

fn column_type_to_oid(col_type: &ColumnType) -> postgres_types::Type {
    match col_type {
        ColumnType::BigInt => postgres_types::Type::INT8,
        ColumnType::BigUInt => postgres_types::Type::INT8,
        ColumnType::Boolean => postgres_types::Type::BOOL,
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
