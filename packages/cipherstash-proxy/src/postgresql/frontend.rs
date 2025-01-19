use std::collections::HashMap;

use super::context::{self, Context};
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
use crate::log::{MAPPER, PROTOCOL};
use crate::postgresql::context::column::Column;
use crate::postgresql::data::literal_from_sql;
use crate::postgresql::messages::Name;
use bytes::BytesMut;
use cipherstash_client::encryption::Plaintext;
use eql_mapper::{self, EqlValue, TableColumn};
use pg_escape::quote_literal;
use sqlparser::ast::{self, Expr, Value};
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
        let (code, mut bytes) = protocol::read_message_with_timeout(
            &mut self.client,
            self.context.client_id,
            connection_timeout,
        )
        .await?;

        if self.encrypt.config.disable_mapping() {
            return Ok(bytes);
        }

        match code.into() {
            Code::Query => {
                if let Some(b) = self.query_handler(&bytes).await? {
                    bytes = b
                }
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
                        let query = Query {
                            statement: content,
                            portal: Name::unnamed(),
                        };
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
                    bytes = b
                }
            }
            Code::Sync => {
                if self.context.schema_changed() {
                    self.encrypt.reload_schema().await;
                }
            }
            _code => {}
        }

        Ok(bytes)
    }

    async fn describe_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let describe = Describe::try_from(bytes)?;
        info!(target: PROTOCOL, "{:?}", describe);
        self.context.set_describe(describe);
        Ok(())
    }

    async fn execute_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let execute = Execute::try_from(bytes)?;
        info!(target: PROTOCOL, "{:?}", execute);
        self.context.set_execute(execute.portal.to_owned());
        Ok(())
    }

    ///
    /// Take the SQL Statement from the Query message
    /// If it contains literal values that map to encrypted configured columns
    /// Encrypt the values
    /// Rewrite the statement
    /// And send it on
    ///
    async fn query_handler(&mut self, bytes: &BytesMut) -> Result<Option<BytesMut>, Error> {
        let mut query = Query::try_from(bytes)?;

        let statement = Parser::new(&DIALECT)
            .try_with_sql(&query.statement)?
            .parse_statement()?;

        let schema_changed =
            eql_mapper::collect_ddl(self.context.table_resolver.clone(), &statement);

        if schema_changed {
            debug!(target: MAPPER, "Changing schema");
            self.context.set_schema_changed();
        }

        let mut requires_rewrite = false;
        info!(target: MAPPER, "Statement {:?}", statement);
        if eql_mapper::requires_type_check(&statement) {
            let typed_statement =
                eql_mapper::type_check(self.context.table_resolver.clone(), &statement).map_err(
                    |e| {
                        info!("{e:?}");
                        MappingError::StatementCouldNotBeTypeChecked
                    },
                )?;

            info!(target: MAPPER, "typed_statement {:?}", typed_statement);

            let literals = &typed_statement.literals;
            info!(target: MAPPER, "Literals {:?}", literals);

            let literal_columns = self.get_literal_columns(&typed_statement)?;

            let plaintexts = Self::get_plaintexts(literals, &literal_columns);

            let encrypted = self.encrypt.encrypt(plaintexts, &literal_columns).await?;

            requires_rewrite = !encrypted.is_empty();

            let encrypted_values = encrypted
                .into_iter()
                .map(|ct| {
                    serde_json::to_string(&ct)
                        .map(|json| ast::Expr::Value(Value::SingleQuotedString(json)))
                })
                .collect::<Result<Vec<_>, _>>()?;

            let original_values_and_replacements = (&typed_statement.literals)
                .iter()
                .map(|(_, original_node)| *original_node)
                .zip(encrypted_values.into_iter())
                .collect::<HashMap<_, _>>();

            let transformed_statement = typed_statement
                .transform(original_values_and_replacements)
                .map_err(|_e| MappingError::StatementCouldNotBeTransformed)?;

            query.statement = transformed_statement.to_string();

            self.context.add_portal(
                query.portal.to_owned(),
                context::Portal::new(
                    context::Statement::new(vec![], vec![], vec![]).into(),
                    vec![], // defaults to all Text if empty
                ),
            );
        }

        if requires_rewrite {
            debug!(target: MAPPER, "Rewrite Query");
            let bytes = BytesMut::try_from(query)?;
            debug!(
                target: MAPPER,
                client_id = self.context.client_id,
                "Mapped params {bytes:?}"
            );
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    fn get_plaintexts(
        literals: &Vec<(EqlValue, &Expr)>,
        literal_columns: &Vec<Option<Column>>,
    ) -> Vec<Option<Plaintext>> {
        literals
            .iter()
            .zip(literal_columns)
            .map(|((_, expr), column)| {
                if let Some(col) = column {
                    match expr {
                        sqlparser::ast::Expr::Value(value) => {
                            literal_from_sql(value.to_string(), &col.postgres_type)
                        }
                        _ => Ok(None),
                    }
                } else {
                    Ok(None)
                }
            })
            .map(|pt| match pt {
                Ok(Some(inner)) => Some(inner.clone()),
                _ => None,
            })
            .collect()
    }

    fn get_literal_columns(
        &mut self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let literal_columns = typed_statement
            .literals
            .iter()
            .map(|(eql_value, _exp)| match eql_value {
                EqlValue(TableColumn { table, column }) => {
                    let identifier = Identifier::from((table, column));
                    debug!(target = MAPPER, "Encrypted literal {:?}", identifier);
                    self.get_column(identifier)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(literal_columns)
    }

    ///
    /// Parse message handler
    /// THIS ONE IS VERY IMPORTANT
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

        debug!(
            target: PROTOCOL,
            client_id = self.context.client_id,
            "Parse: {:?}",
            message
        );

        let statement = Parser::new(&DIALECT)
            .try_with_sql(&message.statement)?
            .parse_statement()?;

        let schema_changed =
            eql_mapper::collect_ddl(self.context.table_resolver.clone(), &statement);

        if schema_changed {
            self.context.set_schema_changed();
        }

        if eql_mapper::requires_type_check(&statement) {
            match eql_mapper::type_check(self.context.table_resolver.clone(), &statement) {
                Ok(typed_statement) => {
                    let param_columns = self.get_param_columns(&typed_statement)?;
                    let projection_columns = self.get_projection_columns(&typed_statement)?;

                    debug!(target: MAPPER,
                        client_id = self.context.client_id,
                        src = "parse_handler",
                        param_columns = ?param_columns
                    );

                    debug!(target: MAPPER,
                        client_id = self.context.client_id,
                        src = "parse_handler",
                        projection_columns = ?projection_columns
                    );

                    // A statement is Encryptable if it has encrypted parameters or result columns
                    if param_columns.is_some() || projection_columns.is_some() {
                        debug!(target: MAPPER,
                            client_id = self.context.client_id,
                            "Encryptable statement");

                        let param_columns = param_columns.unwrap_or(vec![]);
                        let projection_columns = projection_columns.unwrap_or(vec![]);

                        // Capture the parse message param_types
                        // These override the underlying column type
                        let param_types = message.param_types.clone();

                        // Rewrite the type of any encrypted column
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
                }
                Err(error) => {
                    debug!(target: MAPPER,
                        client_id = self.context.client_id,
                        src = "parse_handler",
                        error = ?error
                    );
                    return Err(MappingError::StatementCouldNotBeTypeChecked.into());
                }
            }
        }

        if message.requires_rewrite() {
            debug!(target: MAPPER,
                client_id = self.context.client_id,
                "Rewrite Parse");
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
        debug!(target: PROTOCOL, client_id = self.context.client_id,"Bind: {bind:?}");

        // Only encryptable statements are in the Context
        if let Some(statement) = self.context.get_statement(&bind.prepared_statement) {
            let param_columns = &statement.param_columns;

            let plaintexts = bind.to_plaintext(param_columns, &statement.postgres_param_types)?;

            // TODO: THIS OUTPUTS SENSITIVE DATA
            //       Great for debugging, but not great for production
            debug!(target: MAPPER, client_id = self.context.client_id,"Plaintexts {plaintexts:?}");

            let encrypted = self.encrypt.encrypt(plaintexts, param_columns).await?;

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
            debug!(target: MAPPER, client_id = self.context.client_id,"Rewrite Bind");
            let bytes = BytesMut::try_from(bind)?;
            debug!(
                target: MAPPER,
                client_id = self.context.client_id,
                "Mapped params {bytes:?}"
            );
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    fn get_projection_columns(
        &mut self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Option<Vec<Option<Column>>>, Error> {
        let projection_columns = match &typed_statement.projection {
            Some(projection) => match projection {
                eql_mapper::Projection::WithColumns(columns) => {
                    let mut encryptable = false;
                    let projection_columns = columns
                        .iter()
                        .map(|col| {
                            let eql_mapper::ProjectionColumn { ty, .. } = col;
                            match ty {
                                eql_mapper::Value::Eql(EqlValue(TableColumn { table, column })) => {
                                    let identifier: Identifier = Identifier::from((table, column));
                                    debug!(
                                        target: MAPPER,
                                        client_id = self.context.client_id,
                                        "Encrypted column{:?}",
                                        identifier
                                    );
                                    encryptable = true;
                                    self.get_column(identifier)
                                }
                                _ => Ok(None),
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    if encryptable {
                        Some(projection_columns)
                    } else {
                        None
                    }
                }
                eql_mapper::Projection::Empty => None,
            },
            None => None,
        };
        Ok(projection_columns)
    }

    fn get_param_columns(
        &mut self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Option<Vec<Option<Column>>>, Error> {
        let mut encryptable = false;
        let param_columns = typed_statement
            .params
            .iter()
            .map(|param| match param {
                eql_mapper::Value::Eql(EqlValue(TableColumn { table, column })) => {
                    let identifier = Identifier::from((table, column));
                    debug!(
                        target: MAPPER,
                        client_id = self.context.client_id,
                        "Encrypted parameter {:?}",
                        identifier
                    );
                    encryptable = true;
                    self.get_column(identifier)
                }
                _ => Ok(None),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if encryptable {
            Ok(Some(param_columns))
        } else {
            Ok(None)
        }
    }

    fn get_column(&mut self, identifier: Identifier) -> Result<Option<Column>, Error> {
        match self.encrypt.get_column_config(&identifier) {
            Some(config) => {
                debug!(
                    target: MAPPER,
                    client_id = self.context.client_id,
                    "Configured column {:?}",
                    identifier
                );
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
