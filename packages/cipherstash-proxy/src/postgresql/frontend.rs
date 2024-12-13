use std::collections::HashMap;

use super::context::Context;
use super::messages::FrontendCode as Code;
use super::protocol::{self, Message};
use crate::encrypt::Encrypt;
use crate::eql;
use crate::eql::Identifier;
use crate::error::Error;
use crate::postgresql::CONNECTION_TIMEOUT;
use bytes::BytesMut;
use eql_mapper::{self, EqlColumn, EqlMapperError, TableColumn};
use sqlparser::ast::Value;
use sqlparser::dialect::PostgreSqlDialect;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

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

    async fn parse_handler(&mut self, _message: &Message) -> Result<Option<BytesMut>, Error> {
        Ok(None)
    }

    async fn encrypt_literals(
        &mut self,
        plaintext_literals: Vec<eql::Plaintext>,
    ) -> Result<Vec<Value>, Error> {
        let encrypted = self.encrypt.encrypt_mandatory(plaintext_literals).await?;

        Ok(encrypted
            .into_iter()
            .map(|ct| serde_json::to_string(&ct).map(Value::SingleQuotedString))
            .collect::<Result<Vec<_>, _>>()?)
    }

    async fn bind_handler(&mut self, _message: &Message) -> Result<Option<BytesMut>, Error> {
        Ok(None)
    }
}

fn zip_with_original_value_ref<'ast>(
    typed_statement: &eql_mapper::TypedStatement<'ast>,
    encrypted_literals: Vec<Value>,
) -> HashMap<&'ast Value, Value> {
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

    #[test]
    fn test_parse_handler() {
        trace();
    }
}
