use super::context::{Context, Statement};
use super::error_handler::PostgreSqlErrorHandler;
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
use crate::log::{CONTEXT, MAPPER, PROTOCOL};
use crate::postgresql::context::column::Column;
use crate::postgresql::context::Portal;
use crate::postgresql::data::literal_from_sql;
use crate::postgresql::messages::close::Close;
use crate::postgresql::messages::ready_for_query::ReadyForQuery;
use crate::postgresql::messages::terminate::Terminate;
use crate::postgresql::messages::{Name, Target};
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
use tracing::{debug, error, info, warn};

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

/// The PostgreSQL proxy frontend that handles client-to-server message processing.
///
/// The Frontend intercepts messages from PostgreSQL clients, analyzes SQL statements for
/// encrypted columns, performs encryption transformations, and forwards modified messages
/// to the PostgreSQL server. It implements the PostgreSQL wire protocol and supports both
/// simple queries and extended query protocol (prepared statements).
///
/// # Message Flow
///
/// ```text
/// Client -> Frontend -> Server
///    |         |          |
///    |    [Intercept]     |
///    |    [Parse SQL]     |
///    |    [Encrypt]       |
///    |    [Transform]     |
///    |         |          |
///    +-----> [Forward] ---+
/// ```
///
/// # Key Responsibilities
///
/// - **SQL Analysis**: Parse and type-check SQL statements against schema
/// - **Encryption**: Encrypt literal values and bind parameters for configured columns
/// - **Query Transformation**: Rewrite SQL to use EQL functions for encrypted operations
/// - **Protocol Handling**: Manage PostgreSQL extended query protocol (Parse/Bind/Execute)
/// - **Error Management**: Convert encryption errors to PostgreSQL-compatible error responses
/// - **Context Management**: Track statements, portals, and session state
///
/// # Supported PostgreSQL Messages
///
/// - `Query`: Simple query protocol with SQL string
/// - `Parse`: Prepare statement with parameter placeholders
/// - `Bind`: Bind parameters to prepared statement
/// - `Execute`: Execute bound statement
/// - `Describe`: Describe statement or portal metadata
/// - `Sync`: Synchronization point for extended query protocol
///
/// # Error Handling
///
/// Encryption and mapping errors are converted to appropriate PostgreSQL error responses
/// and sent back to the client. The frontend maintains error state to properly handle
/// the PostgreSQL extended query error recovery protocol.
pub struct Frontend<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    /// Reader for incoming client messages
    client_reader: R,
    /// Sender for outgoing messages to client
    client_sender: Sender,
    /// Writer for forwarding messages to server
    server_writer: W,
    /// Encryption service for column encryption/decryption
    encrypt: Encrypt,
    /// Session context tracking statements, portals, and keyset IDs
    context: Context,
    /// Error state flag for extended query protocol error handling
    error_state: Option<ErrorState>,
}

#[derive(Debug)]
struct ErrorState;

impl<R, W> Frontend<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    /// Creates a new Frontend instance.
    ///
    /// # Arguments
    ///
    /// * `client_reader` - Stream for reading messages from the PostgreSQL client
    /// * `client_sender` - Channel sender for sending messages back to client
    /// * `server_writer` - Stream for writing messages to the PostgreSQL server
    /// * `encrypt` - Encryption service for handling column encryption/decryption
    /// * `context` - Session context for tracking statements and portals
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
            error_state: None,
        }
    }

    /// Main message processing loop for handling client messages.
    ///
    /// Reads a message from the client, processes it based on the PostgreSQL message type,
    /// performs any necessary encryption/transformation, and forwards it to the server.
    ///
    /// # Message Processing Flow
    ///
    /// 1. **Read Message**: Read and parse the PostgreSQL wire protocol message
    /// 2. **Check Mapping**: Skip processing if mapping is disabled
    /// 3. **Handle by Type**: Route to appropriate handler based on message type
    /// 4. **Error Recovery**: Handle extended query protocol error states
    /// 5. **Forward**: Send processed message to PostgreSQL server
    ///
    /// # Error States
    ///
    /// When an error occurs during extended query processing, the frontend enters
    /// error state and discards messages until a Sync message is received, following
    /// the PostgreSQL protocol specification.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful message processing, or an `Error` if a fatal
    /// error occurs that should terminate the connection.
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
        if self.error_state.is_some() {
            warn!(target: PROTOCOL,
                client_id = self.context.client_id,
                error_state = ?self.error_state,
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
                            msg = "Query Handler Error",
                            error = ?err.to_string(),
                        );
                        self.send_error_response(err)?;
                        self.send_ready_for_query()?;
                        return Ok(());
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
                            msg = "Parse Handler Error",
                            error = ?err.to_string(),
                        );
                        self.send_error_response(err)?;
                        return Ok(());
                    }
                }
            }
            Code::Bind => {
                match self.bind_handler(&bytes).await {
                    Ok(Some(mapped)) => bytes = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => match err {
                        Error::Mapping(MappingError::InvalidParameter(_)) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "EncryptError::InvalidParameter",
                            );
                            self.send_error_response(err)?;
                            return Ok(());
                        }
                        Error::Encrypt(EncryptError::UnknownKeysetIdentifier { .. }) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "EncryptError::UnknownKeysetIdentifier",
                            );
                            self.send_error_response(err)?;
                            return Ok(());
                        }
                        _ => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "Bind Error",
                                err = err.to_string()
                            );
                            return Err(err);
                        }
                    },
                }
            }
            Code::Sync => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    ?code,
                );

                if self.context.schema_changed() {
                    self.encrypt.reload_schema().await;
                }

                if self.error_state.is_some() {
                    debug!(target: PROTOCOL,
                        client_id = self.context.client_id,
                        msg = "Ready for Query",
                    );
                    self.send_ready_for_query()?;
                    return Ok(());
                }
            }
            Code::Close => {
                self.close_handler(&bytes).await?;
            }
            code => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    msg = "Passthrough",
                    ?code,
                );
            }
        }

        self.write_to_server(bytes).await?;
        Ok(())
    }

    pub async fn write_to_server(&mut self, bytes: BytesMut) -> Result<(), Error> {
        debug!(target: PROTOCOL, msg = "Write to server", ?bytes);
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

    async fn close_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let close = Close::try_from(bytes)?;
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?close);
        match close.target {
            Target::Portal => self.context.close_portal(&close.name),
            Target::Statement => {
                self.context.close_portal(&close.name);
                // self.context.close_statement(&close.name);
            }
        }
        Ok(())
    }

    async fn execute_handler(&mut self, bytes: &BytesMut) -> Result<(), Error> {
        let execute = Execute::try_from(bytes)?;
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?execute);
        self.context.set_execute(execute.portal.to_owned());
        Ok(())
    }

    /// Handles PostgreSQL Query messages (simple query protocol).
    ///
    /// Processes SQL statements that may contain literal values, encrypting any literals
    /// that correspond to configured encrypted columns and transforming the SQL to use
    /// appropriate EQL functions for encrypted operations.
    ///
    /// # Simple Query Protocol
    ///
    /// The simple query protocol allows sending SQL statements as strings directly,
    /// unlike the extended query protocol which separates parsing, binding, and execution.
    /// This handler supports multiple statements separated by semicolons.
    ///
    /// # Processing Steps
    ///
    /// 1. **Parse Statements**: Split and parse multiple SQL statements
    /// 2. **Check Configuration**: Handle CipherStash-specific SET commands
    /// 3. **Type Check**: Validate statements against database schema
    /// 4. **Encrypt Literals**: Encrypt any literal values in configured columns
    /// 5. **Transform**: Apply EQL transformations to encrypted operations
    /// 6. **Rewrite**: Combine transformed statements back into single query
    ///
    /// # Configuration Commands
    ///
    /// Supports these CipherStash configuration commands:
    /// - `SET CIPHERSTASH.DISABLE_MAPPING = {true|false}` - Enable/disable encryption
    /// - `SET CIPHERSTASH.KEYSET_ID = 'uuid'` - Set encryption keyset ID
    ///
    /// # Returns
    ///
    /// - `Ok(Some(bytes))` - Transformed query that should replace the original
    /// - `Ok(None)` - No transformation needed, forward original query
    /// - `Err(error)` - Processing failed, error should be sent to client
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
                warn!(
                    msg = "SET CIPHERSTASH.DISABLE_MAPPING = {mapping_disabled}",
                    mapping_disabled
                );
            }

            if self.context.unsafe_disable_mapping() {
                warn!(msg = "Encrypted statement mapping is not enabled");
                counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
                continue;
            }

            self.handle_set_keyset(&statement)?;

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
                    debug!(target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Encryptable Statement",
                    );

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

    /// Encrypts literal values found in SQL statements.
    ///
    /// Takes literal values extracted from SQL statements and encrypts those that
    /// correspond to configured encrypted columns using the current keyset ID.
    /// This is used for simple queries where values are embedded directly in SQL.
    ///
    /// # Arguments
    ///
    /// * `typed_statement` - Type-checked statement containing literal value metadata
    /// * `literal_columns` - Column configurations for each literal (Some if encrypted, None if not)
    ///
    /// # Process
    ///
    /// 1. Extract literal values from the typed statement
    /// 2. Convert values to appropriate plaintext types based on column config
    /// 3. Batch encrypt all values using the current keyset ID
    /// 4. Record encryption metrics and timing
    ///
    /// # Returns
    ///
    /// Vector of encrypted values corresponding to each literal, with `None` for
    /// literals that don't require encryption and `Some(EqlEncrypted)` for encrypted values.
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

        let keyset_id = self.context.keyset_identifier();

        let plaintexts = literals_to_plaintext(literal_values, literal_columns)?;

        let start = Instant::now();

        let encrypted = self
            .encrypt
            .encrypt(keyset_id, plaintexts, literal_columns)
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

    /// Handles PostgreSQL Parse messages for the extended query protocol.
    ///
    /// Parse messages contain SQL statements with parameter placeholders ($1, $2, etc.)
    /// that will be bound with actual values in subsequent Bind messages. This handler
    /// analyzes the SQL, performs any necessary transformations for encrypted columns,
    /// and stores the statement metadata for later use.
    ///
    /// # Extended Query Protocol
    ///
    /// The extended query protocol consists of:
    /// 1. **Parse** - Prepare SQL statement with parameters (this handler)
    /// 2. **Bind** - Bind parameter values to prepared statement
    /// 3. **Execute** - Execute the bound statement
    ///
    /// # Statement Naming
    ///
    /// - **Named statements**: Can be reused across multiple Bind/Execute cycles
    /// - **Unnamed statement**: Temporary statement that gets replaced by subsequent Parse messages
    ///
    /// # Processing Steps
    ///
    /// 1. **Parse SQL**: Convert SQL string to AST representation
    /// 2. **Configuration**: Handle CipherStash SET commands (keyset ID, mapping toggle)
    /// 3. **Type Checking**: Validate statement against database schema
    /// 4. **Metadata Collection**: Extract parameter and projection column information
    /// 5. **Transformation**: Apply EQL transformations for encrypted operations
    /// 6. **Storage**: Store statement metadata in context for later Bind operations
    ///
    /// # Parameter Type Handling
    ///
    /// Parameter types can be specified in the Parse message, overriding schema-derived types.
    /// This is important for proper parameter encoding/decoding during Bind operations.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(bytes))` - Modified Parse message with transformed SQL/parameters
    /// - `Ok(None)` - No transformation needed, forward original message
    /// - `Err(error)` - Processing failed, error should be sent to client
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
            warn!(
                msg = "SET CIPHERSTASH.DISABLE_MAPPING = {mapping_disabled}",
                mapping_disabled
            );
        }

        if self.context.unsafe_disable_mapping() {
            warn!(msg = "Encrypted statement mapping is not enabled");
            counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            return Ok(None);
        }

        self.handle_set_keyset(&statement)?;

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
            statement = %statement
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
    /// Handles `SET CIPHERSTASH KEYSET_*` statements
    ///
    /// Returns an error if `SET CIPHERSTASH KEYSET_*` is called and proxy is configured with a `default_keyset_id`
    /// Returns an error if `SET CIPHERSTASH KEYSET_ID` cannot parse the value as a valid UUID
    ///
    fn handle_set_keyset(&mut self, statement: &ast::Statement) -> Result<(), Error> {
        if let Some(keyset_identifier) = self.context.maybe_set_keyset(statement)? {
            debug!(client_id = self.context.client_id, ?keyset_identifier);

            if self.encrypt.config.encrypt.default_keyset_id.is_some() {
                debug!(target: MAPPER,
                    client_id = self.context.client_id,
                    default_keyset_id = ?self.encrypt.config.encrypt.default_keyset_id,
                    ?keyset_identifier
                );
                return Err(EncryptError::UnexpectedSetKeyset.into());
            }
            info!(
                msg = "SET CIPHERSTASH.KEYSET",
                keyset_identifier = keyset_identifier.to_string()
            );
        }

        Ok(())
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

    /// Handles PostgreSQL Bind messages for the extended query protocol.
    ///
    /// Bind messages contain parameter values that are bound to prepared statements
    /// created by previous Parse messages. This handler encrypts parameter values
    /// that correspond to configured encrypted columns and creates a portal for
    /// later execution.
    ///
    /// # Extended Query Protocol Flow
    ///
    /// ```text
    /// Parse    -> Bind         -> Execute
    /// SQL+$1   -> $1='value'   -> Run query
    /// ```
    ///
    /// # Processing Steps
    ///
    /// 1. **Statement Lookup**: Retrieve prepared statement metadata from context
    /// 2. **Parameter Processing**: For each parameter that maps to an encrypted column:
    ///    - Decode parameter value from PostgreSQL wire format
    ///    - Convert to appropriate plaintext type based on column configuration
    ///    - Encrypt using current keyset ID
    ///    - Re-encode in PostgreSQL wire format
    /// 3. **Portal Creation**: Create portal with encryption metadata for Execute phase
    /// 4. **Result Format**: Handle result column format codes for decryption
    ///
    /// # Portal Management
    ///
    /// Portals link Bind operations to Execute operations and carry:
    /// - Statement metadata (parameter/projection column configurations)
    /// - Result format codes (text vs binary encoding)
    /// - Encryption state (whether decryption will be needed)
    ///
    /// # Parameter Encryption
    ///
    /// Only parameters that correspond to configured encrypted columns are processed.
    /// Other parameters are forwarded unchanged to maintain compatibility with
    /// standard PostgreSQL operations.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(bytes))` - Modified Bind message with encrypted parameter values
    /// - `Ok(None)` - No parameter encryption needed, forward original message
    /// - `Err(error)` - Processing failed, error should be sent to client
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
        let keyset_id = self.context.keyset_identifier();
        let plaintexts =
            bind.to_plaintext(&statement.param_columns, &statement.postgres_param_types)?;

        debug!(target: MAPPER, client_id = self.context.client_id, plaintexts = ?plaintexts);
        debug!(target: CONTEXT,
            client_id = self.context.client_id,
            ?keyset_id,
        );

        let start = Instant::now();

        let encrypted = self
            .encrypt
            .encrypt(keyset_id, plaintexts, &statement.param_columns)
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

                let eql_term = eql_term.variant();
                Ok(Some(Column::new(
                    identifier,
                    config,
                    postgres_type,
                    eql_term,
                )))
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
    /// Send an ReadyForQuery to the client and remove error state.
    ///
    fn send_ready_for_query(&mut self) -> Result<(), Error> {
        let message = BytesMut::from(ReadyForQuery);

        debug!(target: PROTOCOL,
            client_id = self.context.client_id,
            msg = "send_ready_for_query",
            ?message,
        );

        self.client_sender.send(message)?;
        self.error_state = None;

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
            Some(col) => literal_from_sql(val, col.eql_term(), col.cast_type()).map_err(|err| {
                debug!(
                    target: MAPPER,
                    msg = "Could not convert literal value",
                    value = ?val,
                    cast_type = ?col.cast_type(),
                    error = err.to_string()
                );
                MappingError::InvalidParameter(Box::new(col.to_owned()))
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

/// Implementation of PostgreSQL error handling for the Frontend component.
impl<R, W> PostgreSqlErrorHandler for Frontend<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    fn client_sender(&mut self) -> &mut Sender {
        &mut self.client_sender
    }

    fn client_id(&self) -> i32 {
        self.context.client_id
    }

    fn send_error_response(&mut self, err: Error) -> Result<(), Error> {
        let error_response = self.error_to_response(err);
        let message = BytesMut::try_from(error_response)?;

        debug!(target: PROTOCOL,
            client_id = self.context.client_id,
            msg = "send_error_response",
            ?message,
        );

        self.client_sender.send(message)?;
        self.error_state = Some(ErrorState); // Frontend-specific: set error state for extended query protocol

        Ok(())
    }
}
