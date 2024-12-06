use super::context::Context;
use super::context::Statement;
use super::messages::bind::Bind;
use super::messages::FrontendCode as Code;
use super::protocol::{self, Message};
use crate::encrypt::Encrypt;
use crate::error::Error;
use crate::postgresql::messages::parse::Parse;
use crate::postgresql::CONNECTION_TIMEOUT;
use bytes::BytesMut;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tracing::{debug, error, info};

const dialect: PostgreSqlDialect = PostgreSqlDialect {};

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
            code => {
                // debug!("Code {code:?}");
            }
        }

        Ok(message.bytes)
    }

    async fn parse_handler(&mut self, message: &Message) -> Result<Option<BytesMut>, Error> {
        debug!("Parse =====================");

        let parse = Parse::try_from(&message.bytes)?;

        let param_types = parse.param_types.clone();

        let ast = Parser::new(&dialect)
            .try_with_sql(&parse.statement)?
            .parse_statement()?;

        // Everything is called Statement and it is a bit annoying
        // Statement contains the parsed ast
        // Should be expanded to include the analyzed and rewritten statement/s
        // etc

        let statement = Statement::new(ast, param_types);

        self.context.add(&parse.name, statement);

        if parse.should_rewrite() {
            let bytes = BytesMut::try_from(parse)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
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

#[cfg(test)]
mod tests {
    use crate::trace;

    use super::Frontend;

    #[test]
    fn test_parse_handler() {
        trace();
    }
}
