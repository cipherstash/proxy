use super::context::{Context, Statement};
use super::messages::bind::Bind;
use super::messages::describe::Describe;
use super::messages::execute::Execute;
use super::messages::parse::Parse;
use super::messages::query::Query;
use super::messages::FrontendCode as Code;
use super::protocol::{self};
use crate::connect::Sender;
use crate::encrypt::Encrypt;
use crate::eql::Identifier;
use crate::error::{EncryptError, Error, MappingError};
use crate::log::{MAPPER, PROTOCOL};
use crate::postgresql::context::column::Column;
use crate::postgresql::context::Portal;
use crate::postgresql::data::literal_from_sql;
use crate::postgresql::messages::error_response::ErrorResponse;
use crate::postgresql::messages::Name;
use crate::prometheus::{
    CLIENTS_BYTES_RECEIVED_TOTAL, ENCRYPTED_VALUES_TOTAL, ENCRYPTION_DURATION_SECONDS,
    ENCRYPTION_ERROR_TOTAL, ENCRYPTION_REQUESTS_TOTAL, SERVER_BYTES_SENT_TOTAL,
    STATEMENTS_ENCRYPTED_TOTAL, STATEMENTS_PASSTHROUGH_TOTAL, STATEMENTS_TOTAL,
    STATEMENTS_UNMAPPABLE_TOTAL,
};
use bytes::BytesMut;
use cipherstash_client::encryption::Plaintext;
use eql_mapper::{self, EqlValue, NodeKey, TableColumn, TypedStatement};
use metrics::{counter, histogram};
use pg_escape::quote_literal;
use serde::Serialize;
use sqlparser::ast::{self, Expr, Value};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use std::collections::HashMap;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, warn};

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

pub struct Frontend<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    client_reader: R,
    client_sender: Sender,
    server_writer: W,
    encrypt: Encrypt,
    context: Context,
    in_error: bool,
}

impl<R, W> Frontend<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    pub fn new(
        client_reader: R,
        client_sender: Sender,
        server_writer: W,
        encrypt: Encrypt,
        context: Context,
    ) -> Self {
        Frontend {
            client_reader,
            client_sender,
            server_writer,
            encrypt,
            context,
            in_error: false,
        }
    }

    pub async fn rewrite(&mut self) -> Result<(), Error> {
        let connection_timeout = self.encrypt.config.database.connection_timeout();
        let (code, mut bytes) = protocol::read_message_with_timeout(
            &mut self.client_reader,
            self.context.client_id,
            connection_timeout,
        )
        .await?;

        let sent: u64 = bytes.len() as u64;
        counter!(CLIENTS_BYTES_RECEIVED_TOTAL).increment(sent);

        if self.encrypt.is_passthrough() {
            self.write_to_server(bytes).await?;
            return Ok(());
        }

        let code = Code::from(code);

        // When an error is detected while processing any extended-query message, the backend issues ErrorResponse, then reads and discards messages until a Sync is reached,
        // https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-EXT-QUERY
        if self.in_error {
            warn!(target: PROTOCOL,
                client_id = self.context.client_id,
                in_error = self.in_error,
                ?code,
            );
            if code != Code::Sync {
                return Ok(());
            }
        }

        match code {
            Code::Query => {
                match self.query_handler(&bytes).await {
                    Ok(Some(mapped)) => bytes = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => {
                        bytes = self.to_database_exception(err)?;
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
                    Err(err) => match err {
                        Error::Mapping(MappingError::InvalidSqlStatement(_)) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "MappingError::SqlParse",
                                error = ?err,
                            );

                            let error_response =
                                ErrorResponse::invalid_sql_statement(err.to_string());

                            self.send_error_response(error_response)?;
                        }
                        Error::Encrypt(EncryptError::UnknownColumn {
                            ref table,
                            ref column,
                        }) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "EncryptError::UnknownColumn",
                            );
                            let error_response =
                                ErrorResponse::unknown_column(err.to_string(), table, column);
                            self.send_error_response(error_response)?;
                        }
                        _ => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "build_frontend_exception",
                            );
                            bytes = self.to_database_exception(err)?;
                        }
                    },
                }
            }
            Code::Bind => {
                match self.bind_handler(&bytes).await {
                    Ok(Some(mapped)) => bytes = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => match err {
                        Error::Mapping(MappingError::InvalidParameter(ref column)) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "EncryptError::InvalidParameter",
                            );

                            let error_response = ErrorResponse::invalid_parameter(
                                err.to_string(),
                                &column.table_name(),
                                &column.column_name(),
                            );

                            self.send_error_response(error_response)?;
                        }
                        _ => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "Bind Error",
                            );
                            return Err(err);
                        }
                    },
                }
            }
            Code::Sync => {
                if self.context.schema_changed() {
                    self.encrypt.reload_schema().await;
                }
                // Clear the error state
                if self.in_error {
                    warn!(target: PROTOCOL,
                        client_id = self.context.client_id,
                        msg = "Clear error state",
                    );
                    self.in_error = false;
                }
            }
            _code => {}
        }

        self.write_to_server(bytes).await?;
        Ok(())
    }

    pub async fn write_to_server(&mut self, bytes: BytesMut) -> Result<(), Error> {
        let sent: u64 = bytes.len() as u64;
        counter!(SERVER_BYTES_SENT_TOTAL).increment(sent);
        self.server_writer.write_all(&bytes).await?;
        Ok(())
    }

    async fn describe_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let describe = Describe::try_from(bytes)?;
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?describe);
        self.context.set_describe(describe);
        Ok(())
    }

    async fn execute_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let execute = Execute::try_from(bytes)?;
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?execute);
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

        let statement = self.parse_statement(&query.statement)?;
        self.check_for_schema_change(&statement);

        if !eql_mapper::requires_type_check(&statement) {
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            return Ok(None);
        }

        let typed_statement = match self.type_check(&statement) {
            Ok(ts) => ts,
            Err(err) => {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Unmappable statement",
                    mapping_errors_enabled = self.encrypt.config.mapping_errors_enabled(),
                    error = err.to_string(),
                );
                if self.encrypt.config.mapping_errors_enabled() {
                    return Err(err);
                } else {
                    return Ok(None);
                };
            }
        };

        let portal = match self.to_encryptable_statement(&typed_statement, vec![])? {
            Some(statement) => {
                if statement.has_literals() || typed_statement.has_nodes_to_wrap() {
                    if let Some(transformed_statement) = self
                        .encrypt_literals(&typed_statement, &statement.literal_columns)
                        .await?
                    {
                        debug!(target: MAPPER,
                            client_id = self.context.client_id,
                            transformed_statement = ?transformed_statement,
                        );
                        query.rewrite(transformed_statement.to_string());
                    };
                }
                counter!(STATEMENTS_ENCRYPTED_TOTAL).increment(1);
                Portal::encrypted(statement.into(), vec![])
            }
            None => {
                debug!(target: MAPPER,
                    client_id = self.context.client_id,
                    msg = "Passthrough Query"
                );
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
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
        literal_columns: &Vec<Column>,
    ) -> Result<Option<ast::Statement>, Error> {
        let literal_values = typed_statement.literal_values();
        let plaintexts = literals_to_plaintext(&literal_values, literal_columns)?;

        let start = Instant::now();

        let encrypted = self
            .encrypt
            .encrypt(plaintexts, literal_columns)
            .await
            .inspect_err(|_| {
                counter!(ENCRYPTION_ERROR_TOTAL).increment(1);
            })?;

        counter!(ENCRYPTION_REQUESTS_TOTAL).increment(1);
        counter!(ENCRYPTED_VALUES_TOTAL).increment(encrypted.len() as u64);

        let duration = Instant::now().duration_since(start);
        histogram!(ENCRYPTION_DURATION_SECONDS).record(duration);

        let encrypted_values = encrypted
            .into_iter()
            .map(|ct| to_json_literal(&ct))
            .collect::<Result<Vec<_>, _>>()?;

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            encrypted_literals = ?encrypted_values
        );

        let original_values_and_replacements = typed_statement
            .literals
            .iter()
            .map(|(_, original_node)| NodeKey::new(*original_node))
            .zip(encrypted_values.into_iter())
            .collect::<HashMap<_, _>>();

        let transformed_statement = typed_statement
            .transform(original_values_and_replacements)
            .map_err(|e| MappingError::StatementCouldNotBeTransformed(e.to_string()))?;

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
            parse = ?message
        );

        let statement = self.parse_statement(&message.statement)?;
        self.check_for_schema_change(&statement);

        if !eql_mapper::requires_type_check(&statement) {
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            return Ok(None);
        }

        let typed_statement = match self.type_check(&statement) {
            Ok(ts) => ts,
            Err(err) => {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Unmappable statement",
                    mapping_errors_enabled = self.encrypt.config.mapping_errors_enabled(),
                    error = err.to_string(),
                );

                if self.encrypt.config.mapping_errors_enabled() {
                    return Err(err);
                } else {
                    return Ok(None);
                };
            }
        };

        // Capture the parse message param_types
        // These override the underlying column type
        let param_types = message.param_types.clone();

        match self.to_encryptable_statement(&typed_statement, param_types)? {
            Some(statement) => {
                if statement.has_literals() || typed_statement.has_nodes_to_wrap() {
                    if let Some(transformed_statement) = self
                        .encrypt_literals(&typed_statement, &statement.literal_columns)
                        .await?
                    {
                        debug!(target: MAPPER,
                            client_id = self.context.client_id,
                            transformed_statement = ?transformed_statement,
                        );

                        message.rewrite_statement(transformed_statement.to_string());
                    };
                }

                counter!(STATEMENTS_ENCRYPTED_TOTAL).increment(1);

                message.rewrite_param_types(&statement.param_columns);
                self.context
                    .add_statement(message.name.to_owned(), statement);
            }
            _ => {
                debug!(target: MAPPER,
                    client_id = self.context.client_id,
                    msg = "Passthrough Parse"
                );
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
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
    /// Parse a SQL statement string into an SqlParser AST
    ///
    fn parse_statement(&mut self, statement: &str) -> Result<ast::Statement, Error> {
        let statement = Parser::new(&DIALECT)
            .try_with_sql(statement)?
            .parse_statement()?;

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            statement = ?statement
        );

        counter!(STATEMENTS_TOTAL).increment(1);

        Ok(statement)
    }

    ///
    /// Check the Statement AST for DDL
    /// Sets a schema changed flag in the Context
    ///
    ///
    fn check_for_schema_change(&self, statement: &ast::Statement) {
        let schema_changed = eql_mapper::collect_ddl(self.context.get_table_resolver(), statement);

        if schema_changed {
            debug!(target: MAPPER,
                client_id = self.context.client_id,
                msg = "schema changed"
            );
            self.context.set_schema_changed();
        }
    }

    ///
    /// Creates a Statement from an EQL Mapper Typed Statement
    /// Returned Statement contains the Column configuration for any encrypted columns in params, literals and projection.
    /// Returns `None` if the Statement is not Encryptable
    ///
    fn to_encryptable_statement(
        &self,
        typed_statement: &TypedStatement<'_>,
        param_types: Vec<i32>,
    ) -> Result<Option<Statement>, Error> {
        let param_columns = self.get_param_columns(typed_statement)?;
        let projection_columns = self.get_projection_columns(typed_statement)?;
        let literal_columns = self.get_literal_columns(typed_statement)?;

        let no_encrypted_param_columns = param_columns.iter().all(|c| c.is_none());
        let no_encrypted_projection_columns = projection_columns.iter().all(|c| c.is_none());

        if (param_columns.is_empty() || no_encrypted_param_columns)
            && (projection_columns.is_empty() || no_encrypted_projection_columns)
            && literal_columns.is_empty()
            && !typed_statement.has_nodes_to_wrap()
        {
            return Ok(None);
        }

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            msg = "Encryptable Statement",
            param_columns = ?param_columns,
            projection_columns = ?projection_columns,
            literal_columns = ?literal_columns,
        );

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

        debug!(target: PROTOCOL, client_id = self.context.client_id, bind = ?bind);

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

    ///
    /// Encrypt Bind Params
    /// Bind holds the params.
    /// Statement holds the column configuration and param types.
    ///
    /// Params are converted to plaintext using the column configuration and any `postgres_param_types` specified on Parse.
    ///
    async fn encrypt_params(
        &mut self,
        bind: &Bind,
        statement: &Statement,
    ) -> Result<Vec<Option<crate::Encrypted>>, Error> {
        let plaintexts =
            bind.to_plaintext(&statement.param_columns, &statement.postgres_param_types)?;

        let start = Instant::now();

        let encrypted = self
            .encrypt
            .encrypt_some(plaintexts, &statement.param_columns)
            .await
            .inspect_err(|_| {
                counter!(ENCRYPTION_ERROR_TOTAL).increment(1);
            })?;

        // Avoid the iter calculation if we can
        if self.encrypt.config.prometheus_enabled() {
            let encrypted_count = encrypted.iter().filter(|e| e.is_some()).count() as u64;

            counter!(ENCRYPTION_REQUESTS_TOTAL).increment(1);
            counter!(ENCRYPTED_VALUES_TOTAL).increment(encrypted_count);

            let duration = Instant::now().duration_since(start);
            histogram!(ENCRYPTION_DURATION_SECONDS).record(duration);
        }

        Ok(encrypted)
    }

    fn type_check<'a>(&self, statement: &'a ast::Statement) -> Result<TypedStatement<'a>, Error> {
        match eql_mapper::type_check(self.context.get_table_resolver(), statement) {
            Ok(typed_statement) => {
                debug!(target: MAPPER,
                    client_id = self.context.client_id,
                    typed_statement = ?typed_statement
                );

                Ok(typed_statement)
            }
            Err(err) => {
                debug!(
                    client_id = self.context.client_id,
                    msg = "Unmappable statement",
                    error = err.to_string()
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::StatementCouldNotBeMapped(err.to_string()).into())
            }
        }
    }

    ///
    /// Maps typed statement projection columns to an Encrypt column configuration
    ///
    /// The returned `Vec` is of `Option<Column>` because the Projection columns are a mix of native and EQL types.
    /// Only EQL colunms will have a configuration. Native types are always None.
    ///
    /// Preserves the ordering and semantics of the projection to reduce the complexity of positional encryption.
    ///
    fn get_projection_columns(
        &self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut projection_columns = vec![];
        if let Some(eql_mapper::Projection::WithColumns(columns)) = &typed_statement.projection {
            for col in columns {
                let eql_mapper::ProjectionColumn { ty, .. } = col;
                let configured_column = match ty {
                    eql_mapper::Value::Eql(EqlValue(TableColumn { table, column })) => {
                        let identifier: Identifier = Identifier::from((table, column));
                        debug!(
                            target: MAPPER,
                            client_id = self.context.client_id,
                            msg = "Configured column",
                            column = ?identifier
                        );
                        let col = self.get_column(identifier)?;
                        Some(col)
                    }
                    _ => None,
                };
                projection_columns.push(configured_column)
            }
        }
        Ok(projection_columns)
    }

    ///
    /// Maps typed statement param columns to an Encrypt column configuration
    ///
    /// The returned `Vec` is of `Option<Column>` because the Param columns are a mix of native and EQL types.
    /// Only EQL colunms will have a configuration. Native types are always None.
    ///
    /// Preserves the ordering and semantics of the projection to reduce the complexity of positional encryption.
    ///
    fn get_param_columns(
        &self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut param_columns = vec![];

        for param in typed_statement.params.iter() {
            let configured_column = match param {
                eql_mapper::Value::Eql(EqlValue(TableColumn { table, column })) => {
                    let identifier = Identifier::from((table, column));

                    debug!(
                        target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Encrypted parameter",
                        column = ?identifier
                    );

                    let col = self.get_column(identifier)?;
                    Some(col)
                }
                _ => None,
            };
            param_columns.push(configured_column);
        }

        Ok(param_columns)
    }

    fn get_literal_columns(
        &self,
        typed_statement: &eql_mapper::TypedStatement<'_>,
    ) -> Result<Vec<Column>, Error> {
        let mut literal_columns = vec![];

        for (eql_value, _) in typed_statement.literals.iter() {
            match eql_value {
                EqlValue(TableColumn { table, column }) => {
                    let identifier = Identifier::from((table, column));
                    debug!(
                        target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Encrypted literal",
                        identifier = ?identifier
                    );
                    let col = self.get_column(identifier)?;
                    literal_columns.push(col);
                }
            }
        }

        Ok(literal_columns)
    }

    ///
    /// Get the column configuration for the Identifier
    /// Returns `EncryptError::UnknownColumn` if configuratiuon cannot be found for the Identified column
    ///
    fn get_column(&self, identifier: Identifier) -> Result<Column, Error> {
        match self.encrypt.get_column_config(&identifier) {
            Some(config) => {
                debug!(
                    target: MAPPER,
                    client_id = self.context.client_id,
                    msg = "Configured column",
                    column = ?identifier
                );
                Ok(Column::new(identifier, config))
            }
            None => {
                debug!(
                    target: MAPPER,
                    client_id = self.context.client_id,
                    msg = "Configured column not found ",
                    ?identifier,
                );
                Err(EncryptError::UnknownColumn {
                    table: identifier.table.to_owned(),
                    column: identifier.column.to_owned(),
                }
                .into())
            }
        }
    }

    ///
    /// Send an ErrorResponse to the client and set the Frontend in error state
    ///
    fn send_error_response(&mut self, error_response: ErrorResponse) -> Result<(), Error> {
        let message = BytesMut::try_from(error_response)?;

        debug!(target: PROTOCOL,
            client_id = self.context.client_id,
            msg = "send_error_response",
            ?message,
        );

        self.client_sender.send(message)?;
        self.in_error = true;

        Ok(())
    }

    /// TODO output err as structured data.
    ///      err can carry any additional context from caller
    fn to_database_exception(&self, err: Error) -> Result<BytesMut, Error> {
        error!(client_id = self.context.client_id, msg = err.to_string(), error = ?err);

        // This *should* be sufficient for escaping error messages as we're only
        // using the string literal, and not identifiers
        let quoted_error = quote_literal(format!("{}", err).as_str());
        let content = format!("DO $$ BEGIN RAISE EXCEPTION {quoted_error}; END; $$;");

        debug!(
            target: MAPPER,
            client_id = self.context.client_id,
            msg = "Frontend exception",
            error = err.to_string()
        );

        let query = Query::new(content);
        let bytes = BytesMut::try_from(query)?;

        Ok(bytes)
    }
}

fn literals_to_plaintext(
    literals: &Vec<&ast::Value>,
    literal_columns: &Vec<Column>,
) -> Result<Vec<Plaintext>, Error> {
    let plaintexts = literals
        .iter()
        .zip(literal_columns)
        .map(|(val, col)| {
            literal_from_sql(val, col.cast_type()).map_err(|err| {
                debug!(
                    target: MAPPER,
                    msg = "Could not convert literal value",
                    value = ?val,
                    cast_type = ?col.cast_type(),
                    error = err.to_string()
                );
                MappingError::InvalidParameter(col.to_owned())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(plaintexts)
}

fn to_json_literal<T>(literal: &T) -> Result<Expr, Error>
where
    T: ?Sized + Serialize,
{
    Ok(serde_json::to_string(literal).map(|json| Expr::Value(Value::SingleQuotedString(json)))?)
}
