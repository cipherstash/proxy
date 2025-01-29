use super::context::{Context, Statement};
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
use crate::postgresql::context::Portal;
use crate::postgresql::data::literal_from_sql;
use crate::postgresql::messages::Name;
use bytes::BytesMut;
use cipherstash_client::encryption::Plaintext;
use eql_mapper::{self, EqlValue, TableColumn, TypedStatement};
use pg_escape::quote_literal;
use serde::Serialize;
use sqlparser::ast::{self, Expr, Value};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use std::collections::HashMap;
use std::fmt::Display;
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
        // TODO: Ideally error messages would be written back to the client as an ErrorResponse
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
                match self.query_handler(&bytes).await {
                    Ok(Some(mapped)) => bytes = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => {
                        bytes = build_frontend_exception(err)?;
                    }
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
                    Ok(Some(mapped)) => bytes = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(e) => {
                        bytes = build_frontend_exception(e)?;
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

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            src = "query_handler",
            statement = ?statement
        );

        let schema_changed = eql_mapper::collect_ddl(self.context.get_table_resolver(), &statement);

        if schema_changed {
            debug!(target: MAPPER,
                client_id = self.context.client_id,
                src = "query_handler",
                msg = "schema changed"
            );
            self.context.set_schema_changed();
        }

        if !eql_mapper::requires_type_check(&statement) {
            return Ok(None);
        }

        let typed_statement = eql_mapper::type_check(self.context.get_table_resolver(), &statement);
        if let Err(error) = typed_statement {
            debug!(target: MAPPER,
                client_id = self.context.client_id,
                error = ?error
            );

            if self.encrypt.config.enable_mapping_errors() {
                return Err(MappingError::StatementCouldNotBeTypeChecked(error.to_string()).into());
            } else {
                return Ok(None);
            }
        }

        let typed_statement = typed_statement.unwrap();
        debug!(target: MAPPER,
            client_id = self.context.client_id,
            src = "query_handler",
            typed_statement = ?typed_statement
        );

        let portal = match self.to_encryptable_statement(&typed_statement, vec![])? {
            Some(statement) => {
                if statement.has_literals() {
                    if let Some(transformed_statement) = self
                        .encrypt_literals(&typed_statement, &statement.literal_columns)
                        .await?
                    {
                        debug!(target: MAPPER,
                            client_id = self.context.client_id,
                            msg = "Transformed Statement",
                            transformed_statement = ?transformed_statement
                        );
                        query.rewrite(transformed_statement.to_string());
                    };
                }

                Portal::encrypted(statement.into(), vec![])
            }
            None => {
                debug!(target: MAPPER,
                    client_id = self.context.client_id,
                    msg = "Query Passthrough"
                );
                Portal::passthrough()
            }
        };

        self.context.add_portal(Name::unnamed(), portal);
        self.context.set_execute(Name::unnamed());

        if query.requires_rewrite() {
            let bytes = BytesMut::try_from(query)?;
            debug!(
                target: MAPPER,
                client_id = self.context.client_id,
                msg = "Rewrite Query",
                bytes = ?bytes
            );
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    ///
    /// Encrypt the literals in the statement using the Column configuration
    /// Returns the transformed statement as an ast::Statement
    ///
    ///
    async fn encrypt_literals(
        &mut self,
        typed_statement: &TypedStatement<'_>,
        literal_columns: &Vec<Option<Column>>,
    ) -> Result<Option<ast::Statement>, Error> {
        let plaintexts = literals_to_plaintext(&typed_statement.literals, literal_columns);

        let encrypted = self.encrypt.encrypt(plaintexts, literal_columns).await?;

        let encrypted_values = encrypted
            .into_iter()
            .map(|ct| to_json_literal(&ct))
            .collect::<Result<Vec<_>, _>>()?;

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            src = "encrypt_literals",
            encrypted_values = ?encrypted_values
        );

        let original_values_and_replacements = typed_statement
            .literals
            .iter()
            .map(|(_, original_node)| *original_node)
            .zip(encrypted_values.into_iter())
            .collect::<HashMap<_, _>>();

        let transformed_statement = typed_statement
            .transform(original_values_and_replacements)
            .map_err(|_e| MappingError::StatementCouldNotBeTransformed)?;

        Ok(Some(transformed_statement))
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

        let schema_changed = eql_mapper::collect_ddl(self.context.get_table_resolver(), &statement);

        if schema_changed {
            self.context.set_schema_changed();
        }

        if eql_mapper::requires_type_check(&statement) {
            match eql_mapper::type_check(self.context.get_table_resolver(), &statement) {
                Ok(typed_statement) => {
                    // Capture the parse message param_types
                    // These override the underlying column type
                    let param_types = message.param_types.clone();

                    if let Some(statement) =
                        self.to_encryptable_statement(&typed_statement, param_types)?
                    {
                        // Rewrite the type of any encrypted column
                        message.rewrite_param_types(&statement.param_columns);

                        self.context
                            .add_statement(message.name.to_owned(), statement);
                    }
                }
                Err(error) => {
                    debug!(target: MAPPER,
                        client_id = self.context.client_id,

                        error = ?error
                    );
                    if self.encrypt.config.enable_mapping_errors() {
                        Err(MappingError::StatementCouldNotBeTypeChecked(
                            error.to_string(),
                        ))?;
                    }
                }
            }
        }

        if message.requires_rewrite() {
            let bytes = BytesMut::try_from(message)?;

            debug!(target: MAPPER,
                client_id = self.context.client_id,
                msg = "Rewrite Parse",
                bytes = ?bytes);

            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    ///
    /// Convert a Typed Statement from Mapper into a Statement
    ///
    ///
    fn to_encryptable_statement(
        &self,
        typed_statement: &TypedStatement<'_>,
        param_types: Vec<i32>,
    ) -> Result<Option<Statement>, Error> {
        let param_columns = self.get_param_columns(typed_statement)?;
        let projection_columns = self.get_projection_columns(typed_statement)?;
        let literal_columns = self.get_literal_columns(typed_statement)?;

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            param_columns = ?param_columns
        );
        debug!(target: MAPPER,
            client_id = self.context.client_id,
            projection_columns = ?projection_columns
        );

        if param_columns.is_none() && projection_columns.is_none() && literal_columns.is_none() {
            return Ok(None);
        }

        debug!(target: MAPPER,
                client_id = self.context.client_id,
                msg = "Encryptable Statement");

        let param_columns = param_columns.unwrap_or(vec![]);
        let projection_columns = projection_columns.unwrap_or(vec![]);
        let literal_columns = literal_columns.unwrap_or(vec![]);

        let statement = Statement::new(
            param_columns.to_owned(),
            projection_columns.to_owned(),
            literal_columns.to_owned(),
            param_types,
        );

        Ok(Some(statement))
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

        let mut portal = Portal::passthrough();

        if let Some(statement) = self.context.get_statement(&bind.prepared_statement) {
            if statement.has_params() {
                let encrypted = self.encrypt_params(&bind, &statement).await?;
                bind.rewrite(encrypted)?;
            }
            if statement.has_projection() {
                portal = Portal::encrypted(statement, bind.result_columns_format_codes.to_owned());
            }
        };

        debug!(target: MAPPER, client_id = self.context.client_id, portal = ?portal);
        self.context.add_portal(bind.portal.to_owned(), portal);

        if bind.requires_rewrite() {
            let bytes = BytesMut::try_from(bind)?;
            debug!(
                target: MAPPER,
                client_id = self.context.client_id,
                msg = "Rewrite Bind",
                bytes = ?bytes
            );

            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    async fn encrypt_params(
        &mut self,
        bind: &Bind,
        statement: &Statement,
    ) -> Result<Vec<Option<crate::Ciphertext>>, Error> {
        let plaintexts =
            bind.to_plaintext(&statement.param_columns, &statement.postgres_param_types)?;
        let encrypted = self
            .encrypt
            .encrypt(plaintexts, &statement.param_columns)
            .await?;
        Ok(encrypted)
    }

    fn get_projection_columns(
        &self,
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
        &self,
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

    fn get_literal_columns(
        &self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Option<Vec<Option<Column>>>, Error> {
        if typed_statement.literals.is_empty() {
            return Ok(None);
        }
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
        Ok(Some(literal_columns))
    }

    fn get_column(&self, identifier: Identifier) -> Result<Option<Column>, Error> {
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

fn build_frontend_exception<E: Display>(err: E) -> Result<BytesMut, Error> {
    // This *should* be sufficient for escaping error messages as we're only
    // using the string literal, and not identifiers
    let quoted_error = quote_literal(format!("[CipherStash] {}", err).as_str());
    let content = format!("DO $$ begin raise exception {quoted_error}; END; $$;");

    let query = Query::new(content);
    let bytes = BytesMut::try_from(query)?;
    debug!(
        "frontend sending an exception-raising message: {:?}",
        &bytes
    );
    Ok(bytes)
}

fn literals_to_plaintext(
    literals: &Vec<(EqlValue, &Expr)>,
    literal_columns: &Vec<Option<Column>>,
) -> Vec<Option<Plaintext>> {
    literals
        .iter()
        .zip(literal_columns)
        .map(|((_, expr), column)| {
            column.as_ref().and_then(|col| {
                if let sqlparser::ast::Expr::Value(value) = expr {
                    literal_from_sql(value.to_string(), &col.postgres_type)
                        .ok()
                        .flatten()
                } else {
                    None
                }
            })
        })
        .collect()
}

fn to_json_literal<T>(literal: &T) -> Result<Expr, Error>
where
    T: ?Sized + Serialize,
{
    Ok(serde_json::to_string(literal).map(|json| Expr::Value(Value::SingleQuotedString(json)))?)
}
