use std::collections::HashMap;

use super::context::Context;
use super::messages::bind::Bind;
use super::messages::FrontendCode as Code;
use super::protocol::{self, Message};
use crate::encrypt::Encrypt;
use crate::eql;
use crate::eql::Identifier;
use crate::error::Error;
use crate::postgresql::messages::parse::Parse;
use crate::postgresql::{context, CONNECTION_TIMEOUT};
use bytes::BytesMut;
use eql_mapper::{self, EqlColumn, EqlMapperError, TableColumn};
use sqlparser::ast::Value;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tracing::{debug, error, info};

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

        let mut message =
            timeout(CONNECTION_TIMEOUT, protocol::read_message(&mut self.client)).await??;

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
                typed_statement.get_projection_columns().map(Vec::from),
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
    ) -> Result<Vec<Value>, Error> {
        let encrypted = self.encrypt.encrypt_mandatory(plaintext_literals).await?;

        Ok(encrypted
            .into_iter()
            .map(|ct| serde_json::to_string(&ct).map(|json| Value::SingleQuotedString(json)))
            .collect::<Result<Vec<_>, _>>()?)
    }

    async fn bind_handler(&mut self, message: &Message) -> Result<Option<BytesMut>, Error> {
        let mut bind = Bind::try_from(&message.bytes)?;

        if let Some(statement) = self.context.get(&bind.prepared_statement) {
            info!("Statement {statement:?}");
        } else {
            error!("No statement {:?} exists", &bind.prepared_statement);
            // return Ok(None);
        }

        let params = bind.to_plaintext()?;
        let encrypted = self.encrypt.encrypt(params).await?;

        bind.from_ciphertext(encrypted)?;

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
    encrypted_literals: Vec<Value>,
) -> HashMap<&'ast Value, Value> {
    (&typed_statement.literals)
        .into_iter()
        .map(|(_, original_node)| *original_node)
        .zip(encrypted_literals.into_iter())
        .collect::<HashMap<_, _>>()
}

fn convert_value_nodes_to_eql_plaintext(
    typed_statement: &eql_mapper::TypedStatement<'_>,
) -> Result<Vec<eql::Plaintext>, EqlMapperError> {
    (&typed_statement.literals)
        .iter()
        .map(|(EqlColumn(TableColumn { table, column }), value)| {
            if let Some(plaintext) = match value {
                Value::Number(number, _) => Some(number.to_string()),
                Value::SingleQuotedString(s) => Some(s.to_owned()),
                Value::Boolean(b) => Some(b.to_string()),
                Value::Null => None,
                _ => None,
            } {
                Ok(eql::Plaintext {
                    identifier: Identifier::from((table, column)),
                    plaintext,
                    version: 1,
                    for_query: None,
                })
            } else {
                Err(EqlMapperError::UnsupportedValueVariant(value.to_string()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::trace;

    use super::Frontend;

    #[test]
    fn test_parse_handler() {
        trace();
    }
}
