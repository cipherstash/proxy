use std::collections::HashMap;

use super::context::{self, Context};
use super::messages::parse::Parse;
use super::messages::FrontendCode as Code;
use super::protocol::{self, Message};
use crate::encrypt::Encrypt;
use crate::eql;
use crate::eql::Identifier;
use crate::error::Error;
use crate::log::DEVELOPMENT;
use bytes::BytesMut;
use eql_mapper::{self, EqlMapperError, EqlValue, TableColumn};
use sqlparser::ast::{CastKind, DataType, Expr, Value};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::warn;

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
    pub fn new(client: C, server: S, encrypt: Encrypt) -> Self {
        let context = Context::new();
        Frontend {
            client,
            server,
            encrypt,
            context,
        }
    }

    pub async fn rewrite(&mut self) -> Result<(), Error> {
        if self.encrypt.config.disable_mapping() {
            warn!(DEVELOPMENT, "Mapping is not enabled");
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
        // debug!("[frontend] read");

        let mut message = protocol::read_message_with_timeout(&mut self.client).await?;

        match message.code.into() {
            Code::Query => {}
            Code::Parse => {
                if let Some(bytes) = self.parse_handler(&message).await? {
                    message.bytes = bytes;
                }
            }
            Code::Bind => {
                if let Some(bytes) = self.bind_handler(&message).await? {
                    message.bytes = bytes;
                }
            }
            _code => {}
        }

        Ok(message.bytes)
    }

    async fn parse_handler(&mut self, message: &Message) -> Result<Option<BytesMut>, Error> {
        let mut parse = Parse::try_from(&message.bytes)?;

        let statement = Parser::new(&DIALECT)
            .try_with_sql(&parse.statement)?
            .parse_statement()?;

        let typed_statement = eql_mapper::type_check(self.encrypt.schema.load(), &statement)?;

        let plaintext_literals: Vec<eql::Plaintext> =
            convert_value_nodes_to_eql_plaintext(&typed_statement)?;

        let replacement_literal_values = self.encrypt_literals(plaintext_literals).await?;

        let original_values_and_replacements =
            zip_with_original_value_ref(&typed_statement, replacement_literal_values);

        let transformed_statement = typed_statement.transform(original_values_and_replacements)?;

        parse.statement = transformed_statement.to_string().clone();

        self.context.add(
            crate::postgresql::messages::Destination::Unnamed,
            context::Statement::new(
                transformed_statement,
                parse.param_types.clone(),
                typed_statement.params.clone(),
                typed_statement.statement_type.clone(),
            ),
        );

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

    async fn bind_handler(&mut self, _message: &Message) -> Result<Option<BytesMut>, Error> {
        Ok(None)
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
