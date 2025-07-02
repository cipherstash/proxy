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
use crate::postgresql::messages::terminate::Terminate;
use crate::postgresql::messages::Name;
use crate::prometheus::{
    CLIENTS_BYTES_RECEIVED_TOTAL, ENCRYPTED_VALUES_TOTAL, ENCRYPTION_DURATION_SECONDS,
    ENCRYPTION_ERROR_TOTAL, ENCRYPTION_REQUESTS_TOTAL, SERVER_BYTES_SENT_TOTAL,
    STATEMENTS_ENCRYPTED_TOTAL, STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL,
    STATEMENTS_PASSTHROUGH_TOTAL, STATEMENTS_TOTAL, STATEMENTS_UNMAPPABLE_TOTAL,
};
use crate::EqlEncrypted;
use bytes::BytesMut;
use cipherstash_client::encryption::Plaintext;
use eql_mapper::{self, EqlMapperError, EqlTerm, TableColumn, TypeCheckedStatement};
use metrics::{counter, histogram};
use pg_escape::quote_literal;
use postgres_types::Type;
use serde::Serialize;
use sqltk::parser::ast::{self, Value};
use sqltk::parser::dialect::PostgreSqlDialect;
use sqltk::parser::parser::Parser;
use sqltk::NodeKey;
use std::collections::HashMap;
use std::sync::Arc;
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
        let (code, mut bytes) = protocol::read_message(
            &mut self.client_reader,
            self.context.client_id,
            connection_timeout,
        )
        .await?;

        let sent: u64 = bytes.len() as u64;
        counter!(CLIENTS_BYTES_RECEIVED_TOTAL).increment(sent);

        if self.encrypt.config.mapping_disabled() {
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
                        warn!(
                                client_id = self.context.client_id,
                                msg = "Query Error",
                                error = ?err.to_string(),
                        );
                        self.error_handler(err).await?;
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
                    Err(err) => {
                        warn!(
                                client_id = self.context.client_id,
                                msg = "Parse Error",
                                error = ?err.to_string(),
                        );
                        self.error_handler(err).await?;
                    }
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

    pub async fn error_handler(&mut self, err: Error) -> Result<(), Error> {
        let error_response = match err {
            Error::Mapping(err) => ErrorResponse::invalid_sql_statement(err.to_string()),
            Error::Encrypt(EncryptError::UnknownColumn {
                ref table,
                ref column,
            }) => ErrorResponse::unknown_column(err.to_string(), table, column),
            _ => ErrorResponse::system_error(err.to_string()),
        };
        self.send_error_response(error_response)?;
        Ok(())
    }

    pub async fn write_to_server(&mut self, bytes: BytesMut) -> Result<(), Error> {
        let sent: u64 = bytes.len() as u64;
        counter!(SERVER_BYTES_SENT_TOTAL).increment(sent);
        self.server_writer.write_all(&bytes).await?;
        Ok(())
    }

    pub async fn terminate(&mut self) -> Result<(), Error> {
        debug!(target: PROTOCOL, msg = "Terminate server connection");
        let bytes = Terminate::message();
        self.write_to_server(bytes).await?;
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
        self.context.start_session();

        let mut query = Query::try_from(bytes)?;

        // Simple Query may contain many statements
        let parsed_statements = self.parse_statements(&query.statement)?;
        let mut transformed_statements = vec![];

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            statements = parsed_statements.len(),
        );

        let mut portal = Portal::passthrough();
        let mut encrypted = false;

        for statement in parsed_statements {
            if let Some(mapping_disabled) =
                self.context.maybe_set_unsafe_disable_mapping(&statement)
            {
                warn!("SET CIPHERSTASH.DISABLE_MAPPING" = mapping_disabled);
            }

            if self.context.unsafe_disable_mapping() {
                warn!(msg = "Encrypted statement mapping is not enabled");
                counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
                continue;
            }

            self.check_for_schema_change(&statement);

            if !eql_mapper::requires_type_check(&statement) {
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
                continue;
            }

            let typed_statement = match self.type_check(&statement) {
                Ok(ts) => ts,
                Err(err) => {
                    if self.encrypt.config.mapping_errors_enabled() {
                        return Err(err);
                    } else {
                        return Ok(None);
                    };
                }
            };

            match self.to_encryptable_statement(&typed_statement, vec![])? {
                Some(statement) => {
                    if typed_statement.requires_transform() {
                        let encrypted_literals = self
                            .encrypt_literals(&typed_statement, &statement.literal_columns)
                            .await?;

                        if let Some(transformed_statement) = self
                            .transform_statement(&typed_statement, &encrypted_literals)
                            .await?
                        {
                            debug!(target: MAPPER,
                                client_id = self.context.client_id,
                                transformed_statement = ?transformed_statement,
                            );

                            transformed_statements.push(transformed_statement);
                            encrypted = true;
                        }
                    }
                    debug!(target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Encrypted Statement"
                    );
                    counter!(STATEMENTS_ENCRYPTED_TOTAL).increment(1);

                    // Set Encrypted portal
                    portal = Portal::encrypted(Arc::new(statement));
                }
                None => {
                    debug!(target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Passthrough Statement"
                    );
                    counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
                    transformed_statements.push(statement);
                }
            };
        }

        self.context.add_portal(Name::unnamed(), portal);
        self.context.set_execute(Name::unnamed());

        if encrypted {
            let transformed_statement = transformed_statements
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(";");

            query.rewrite(transformed_statement.to_string());

            let bytes = BytesMut::try_from(query)?;
            debug!(
                target: MAPPER,
                client_id = self.context.client_id,
                msg = "Rewrite Query",
                transformed_statement = transformed_statement.to_string(),
                bytes = ?bytes,
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
        typed_statement: &TypeCheckedStatement<'_>,
        literal_columns: &Vec<Option<Column>>,
    ) -> Result<Vec<Option<EqlEncrypted>>, Error> {
        let literal_values = typed_statement.literal_values();
        if literal_values.is_empty() {
            debug!(target: MAPPER,
                client_id = self.context.client_id,
                msg = "No literals to encrypt"
            );
            return Ok(vec![]);
        }

        let plaintexts = literals_to_plaintext(literal_values, literal_columns)?;

        let start = Instant::now();

        let encrypted = self
            .encrypt
            .encrypt(plaintexts, literal_columns)
            .await
            .inspect_err(|_| {
                counter!(ENCRYPTION_ERROR_TOTAL).increment(1);
            })?;

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            ?literal_columns,
            ?encrypted
        );

        counter!(ENCRYPTION_REQUESTS_TOTAL).increment(1);
        counter!(ENCRYPTED_VALUES_TOTAL).increment(encrypted.len() as u64);

        let duration = Instant::now().duration_since(start);
        histogram!(ENCRYPTION_DURATION_SECONDS).record(duration);

        Ok(encrypted)
    }

    ///
    /// Transforms a typed statement
    ///  - rewrites any encrypted literal values
    ///  - wraps any nodes in appropriate EQL function
    ///
    async fn transform_statement(
        &mut self,
        typed_statement: &TypeCheckedStatement<'_>,
        encrypted_literals: &Vec<Option<EqlEncrypted>>,
    ) -> Result<Option<ast::Statement>, Error> {
        // Convert literals to ast Expr
        let mut encrypted_expressions = vec![];
        for encrypted in encrypted_literals {
            let e = match encrypted {
                Some(en) => Some(to_json_literal_value(&en)?),
                None => None,
            };
            encrypted_expressions.push(e);
        }

        // Map encrypted literal values back to the Expression nodes.
        // Filter out the Null/None values to only include literals that have been encrypted
        let encrypted_nodes = typed_statement
            .literals
            .iter()
            .zip(encrypted_expressions.into_iter())
            .filter_map(|((_, original_node), en)| en.map(|en| (NodeKey::new(*original_node), en)))
            .collect::<HashMap<_, _>>();

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            literals = encrypted_nodes.len(),
        );

        if !typed_statement.requires_transform() {
            return Ok(None);
        }

        let transformed_statement = typed_statement
            .transform(encrypted_nodes)
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
        self.context.start_session();

        let mut message = Parse::try_from(bytes)?;

        debug!(
            target: PROTOCOL,
            client_id = self.context.client_id,
            parse = ?message
        );

        let statement = self.parse_statement(&message.statement)?;

        if let Some(mapping_disabled) = self.context.maybe_set_unsafe_disable_mapping(&statement) {
            warn!("SET CIPHERSTASH.DISABLE_MAPPING" = mapping_disabled);
        }

        if self.context.unsafe_disable_mapping() {
            warn!(msg = "Encrypted statement mapping is not enabled");
            counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            return Ok(None);
        }

        self.check_for_schema_change(&statement);

        if !eql_mapper::requires_type_check(&statement) {
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            return Ok(None);
        }

        let typed_statement = match self.type_check(&statement) {
            Ok(ts) => ts,
            Err(err) => {
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
                if typed_statement.requires_transform() {
                    let encrypted_literals = self
                        .encrypt_literals(&typed_statement, &statement.literal_columns)
                        .await?;

                    if let Some(transformed_statement) = self
                        .transform_statement(&typed_statement, &encrypted_literals)
                        .await?
                    {
                        debug!(target: MAPPER,
                            client_id = self.context.client_id,
                            transformed_statement = ?transformed_statement,
                        );

                        message.rewrite_statement(transformed_statement.to_string());
                    }
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
    /// Parse a SQL String potentially containing multiple statements into parsed SqlParser AST
    ///
    fn parse_statements(&mut self, statement: &str) -> Result<Vec<ast::Statement>, Error> {
        let statement = Parser::new(&DIALECT)
            .try_with_sql(statement)?
            .parse_statements()?;

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            statement = ?statement
        );

        counter!(STATEMENTS_TOTAL).increment(statement.len() as u64);

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
        typed_statement: &TypeCheckedStatement<'_>,
        param_types: Vec<i32>,
    ) -> Result<Option<Statement>, Error> {
        let param_columns = self.get_param_columns(typed_statement)?;
        let projection_columns = self.get_projection_columns(typed_statement)?;
        let literal_columns = self.get_literal_columns(typed_statement)?;

        let no_encrypted_param_columns = param_columns.iter().all(|c| c.is_none());
        let no_encrypted_projection_columns = projection_columns.iter().all(|c| c.is_none());

        if (param_columns.is_empty() || no_encrypted_param_columns)
            && (projection_columns.is_empty() || no_encrypted_projection_columns)
            && !typed_statement.requires_transform()
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
        if self.context.unsafe_disable_mapping() {
            warn!(msg = "Encrypted statement mapping is not enabled");
            counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            return Ok(None);
        }

        let mut bind = Bind::try_from(bytes)?;

        debug!(target: PROTOCOL, client_id = self.context.client_id, bind = ?bind);

        let mut portal = Portal::passthrough();

        if let Some(statement) = self.context.get_statement(&bind.prepared_statement) {
            debug!(target:MAPPER, client_id = self.context.client_id, ?statement);

            if statement.has_params() {
                let encrypted = self.encrypt_params(&bind, &statement).await?;
                bind.rewrite(encrypted)?;
            }
            if statement.has_projection() {
                portal = Portal::encrypted_with_format_codes(
                    statement,
                    bind.result_columns_format_codes.to_owned(),
                );
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
    ) -> Result<Vec<Option<crate::EqlEncrypted>>, Error> {
        let plaintexts =
            bind.to_plaintext(&statement.param_columns, &statement.postgres_param_types)?;

        debug!(target: MAPPER, client_id = self.context.client_id, plaintexts = ?plaintexts);

        let start = Instant::now();

        let encrypted = self
            .encrypt
            .encrypt(plaintexts, &statement.param_columns)
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

    fn type_check<'a>(
        &self,
        statement: &'a ast::Statement,
    ) -> Result<TypeCheckedStatement<'a>, Error> {
        match eql_mapper::type_check(self.context.get_table_resolver(), statement) {
            Ok(typed_statement) => {
                debug!(target: MAPPER,
                    client_id = self.context.client_id,
                    typed_statement = ?typed_statement
                );

                Ok(typed_statement)
            }
            Err(EqlMapperError::InternalError(str)) => {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Internal Error in EQL Mapper",
                    mapping_errors_enabled = self.encrypt.config.mapping_errors_enabled(),
                    error = str,
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::Internal(str).into())
            }
            Err(err) => {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Unmappable statement",
                    mapping_errors_enabled = self.encrypt.config.mapping_errors_enabled(),
                    error = err.to_string(),
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::StatementCouldNotBeTypeChecked(err.to_string()).into())
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
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut projection_columns = vec![];

        for col in typed_statement.projection.columns() {
            let eql_mapper::ProjectionColumn { ty, .. } = col;
            let configured_column = match &**ty {
                eql_mapper::Type::Value(eql_mapper::Value::Eql(eql_term)) => {
                    let TableColumn { table, column } = eql_term.table_column();
                    let identifier: Identifier = Identifier::from((table, column));

                    debug!(
                        target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Configured column",
                        column = ?identifier,
                        ?eql_term,
                    );
                    self.get_column(identifier, eql_term)?
                }
                _ => None,
            };
            projection_columns.push(configured_column)
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
    ///
    fn get_param_columns(
        &self,
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut param_columns = vec![];

        for param in typed_statement.params.iter() {
            let configured_column = match param {
                (_, eql_mapper::Value::Eql(eql_term)) => {
                    let TableColumn { table, column } = eql_term.table_column();
                    let identifier = Identifier::from((table, column));

                    debug!(
                        target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Encrypted parameter",
                        column = ?identifier,
                        ?eql_term,
                    );

                    self.get_column(identifier, eql_term)?
                }
                _ => None,
            };
            param_columns.push(configured_column);
        }

        Ok(param_columns)
    }

    fn get_literal_columns(
        &self,
        typed_statement: &eql_mapper::TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut literal_columns = vec![];

        for (eql_term, _) in typed_statement.literals.iter() {
            let TableColumn { table, column } = eql_term.table_column();
            let identifier = Identifier::from((table, column));

            debug!(
                target: MAPPER,
                client_id = self.context.client_id,
                msg = "Encrypted literal",
                column = ?identifier,
                ?eql_term,
            );
            let col = self.get_column(identifier, eql_term)?;
            if col.is_some() {
                literal_columns.push(col);
            }
        }

        Ok(literal_columns)
    }

    ///
    /// Get the column configuration for the Identifier
    /// Returns `EncryptError::UnknownColumn` if configuration cannot be found for the Identified column
    /// if mapping enabled, and None if mapping is disabled. It'll log a warning either way.
    fn get_column(
        &self,
        identifier: Identifier,
        eql_term: &EqlTerm,
    ) -> Result<Option<Column>, Error> {
        match self.encrypt.get_column_config(&identifier) {
            Some(config) => {
                debug!(
                    target: MAPPER,
                    client_id = self.context.client_id,
                    msg = "Configured column",
                    column = ?identifier
                );

                // IndexTerm::SteVecSelector

                let postgres_type = if matches!(eql_term, EqlTerm::JsonPath(_)) {
                    Some(Type::JSONPATH)
                } else {
                    None
                };

                Ok(Some(Column::new(identifier, config, postgres_type)))
            }
            None => {
                warn!(
                    target: MAPPER,
                    client_id = self.context.client_id,
                    msg = "Configured column not found. Encryption configuration may have been deleted.",
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
        let quoted_error = quote_literal(format!("{err}").as_str());
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
    literals: &Vec<(EqlTerm, &ast::Value)>,
    literal_columns: &Vec<Option<Column>>,
) -> Result<Vec<Option<Plaintext>>, Error> {
    let plaintexts = literals
        .iter()
        .zip(literal_columns)
        .map(|((_, val), col)| match col {
            Some(col) => literal_from_sql(val, col.cast_type()).map_err(|err| {
                debug!(
                    target: MAPPER,
                    msg = "Could not convert literal value",
                    value = ?val,
                    cast_type = ?col.cast_type(),
                    error = err.to_string()
                );
                MappingError::InvalidParameter(col.to_owned())
            }),
            None => Ok(None),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(plaintexts)
}

fn to_json_literal_value<T>(literal: &T) -> Result<Value, Error>
where
    T: ?Sized + Serialize,
{
    Ok(serde_json::to_string(literal).map(Value::SingleQuotedString)?)
}
