use super::context::{self, Column, Context};
use super::messages::bind::Bind;
use super::messages::describe::Describe;
use super::messages::execute::Execute;
use super::messages::parse::Parse;
use super::messages::query::Query;
use super::messages::FrontendCode as Code;
use super::protocol::{self};
use crate::encrypt::Encrypt;
use crate::eql::Identifier;
use crate::error::{EncryptError, Error, MappingError};
use crate::log::MAPPER;
use bytes::BytesMut;
use eql_mapper::{self, EqlValue, TableColumn};
use pg_escape::quote_literal;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info, warn};

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
            Code::Query => {
                self.query_handler(&bytes).await?;
            }
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
        info!(target = MAPPER, "{:?}", describe);
        self.context.describe(describe);
        Ok(())
    }

    async fn execute_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let execute = Execute::try_from(bytes)?;
        info!(target = MAPPER, "{:?}", execute);
        self.context.execute(execute.portal.to_owned());
        Ok(())
    }

    async fn query_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let query = Query::try_from(bytes)?;

        let statement = Parser::new(&DIALECT)
            .try_with_sql(&query.statement)?
            .parse_statement()?;

        info!(target = MAPPER, "Statement {:?}", statement);

        if eql_mapper::requires_type_check(&statement) {
            let typed_statement = eql_mapper::type_check(self.encrypt.schema.load(), &statement)
                .map_err(|_| MappingError::StatementCouldNotBeTypeChecked)?;

            info!(target = MAPPER, "typed_statement {:?}", typed_statement);
            info!(target = MAPPER, "Literals {:?}", typed_statement.literals);
        }

        Ok(())
    }

    ///
    /// Parse message handler
    /// THIS ONE IS VER IMPORTANT
    ///
    /// Parse messages contain the actual SQL Statement
    /// Handler handles
    ///  - parse sql into Statement AST
    ///  - type check the statement against the schema
    ///  - collect parameter and projection metadata
    ///  - add meta data to the Context
    ///
    /// Context is keyed by message.name and message.name may be empty (Unnamed)
    ///
    /// A named statement is essentially a stored procedure.
    /// Once a named statement has been successfully parsed, the client may refer to the statement
    /// by name in the Bind step.
    ///
    /// There is in effect only one Unnamed Statement.
    /// Subsequent parse messages with an empty name overide the Unnamed Statement.
    ///
    /// This is how keys work, if you send the same key with a different statement then you have a different statement?
    ///
    /// The name is used as key when the Statement is stored in the Context.
    ///
    ///
    ///
    async fn parse_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        let mut message = Parse::try_from(bytes)?;

        warn!(target = MAPPER, "Parse: {:?}", message);

        let statement = Parser::new(&DIALECT)
            .try_with_sql(&message.statement)?
            .parse_statement()?;

        if eql_mapper::requires_type_check(&statement) {
            let typed_statement = eql_mapper::type_check(self.encrypt.schema.load(), &statement)
                .map_err(|_e| MappingError::StatementCouldNotBeTypeChecked)?;

            let param_columns = self.get_param_columns(&typed_statement)?;
            let projection_columns = self.get_projection_columns(&typed_statement)?;

            // Capture the current param_types
            let param_types = message.param_types.clone();

            message.rewrite_param_types(&param_columns);

            self.context.add_statement(
                message.name.to_owned(),
                context::Statement::new(
                    param_columns.clone(),
                    projection_columns.clone(),
                    param_types,
                ),
            );
        }

        if message.requires_rewrite() {
            debug!(target: MAPPER, "Rewrite Parse");
            let bytes = BytesMut::try_from(message)?;
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
        debug!(target: MAPPER, "Bind: {bind:?}");

        if let Some(statement) = self.context.get_statement(&bind.prepared_statement) {
            let param_columns = &statement.param_columns;
            let plaintexts = bind.to_plaintext(&param_columns, &statement.postgres_param_types)?;

            let encrypted = self.encrypt.encrypt(plaintexts, &param_columns).await?;

            bind.rewrite(encrypted)?;

            self.context.add_portal(
                bind.portal.to_owned(),
                context::Portal::new(
                    statement.clone(),
                    bind.result_columns_format_codes.to_owned(),
                ),
            );
        }

        if bind.requires_rewrite() {
            debug!(target: MAPPER, "Rewrite Bind");
            let bytes = BytesMut::try_from(bind)?;
            debug!(target = MAPPER, "Mapped params {bytes:?}");
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
}
