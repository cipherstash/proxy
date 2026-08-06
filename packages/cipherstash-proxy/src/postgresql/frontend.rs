use super::context::phase_timing::PhaseTimer;
use super::context::{Context, SessionId, Statement};
use super::error_handler::PostgreSqlErrorHandler;
use super::messages::bind::Bind;
use super::messages::parse::Parse;
use super::messages::query::Query;
use super::parser::SqlParser;
use crate::connect::Sender;
use crate::error::{EncryptError, Error, MappingError};
use crate::log::{MAPPER, PROTOCOL};
use crate::postgresql::context::column::Column;
use crate::postgresql::context::statement::{
    output_params_from_plan, OutputParam, OutputParamSource,
};
use crate::postgresql::context::statement_metadata::{ProtocolType, StatementType};
use crate::postgresql::context::Portal;
use crate::postgresql::data::{
    compose_json_selector_path, json_value_selector_plaintext, literal_from_sql, literal_json_value,
};
use crate::postgresql::messages::Name;
use crate::prometheus::{
    CLIENTS_BYTES_RECEIVED_TOTAL, ENCRYPTED_VALUES_TOTAL, ENCRYPTION_DURATION_SECONDS,
    ENCRYPTION_ERROR_TOTAL, ENCRYPTION_REQUESTS_TOTAL, SERVER_BYTES_SENT_TOTAL,
    STATEMENTS_ENCRYPTED_TOTAL, STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL,
    STATEMENTS_PASSTHROUGH_TOTAL, STATEMENTS_UNMAPPABLE_TOTAL,
};
use crate::proxy::EncryptionService;
use crate::{EqlOutput, EqlQueryPayload};
use cipherstash_client::encryption::Plaintext;
use eql_mapper::{self, EqlMapperError, EqlTermVariant, JsonSelectorSegment, TypeCheckedStatement};
use metrics::{counter, histogram};
use pg_proto::{
    codec::{
        Backend as BackendDirection, BackendMessage, Close, Describe, DescribeTarget, Execute,
        Frontend as FrontendDirection, FrontendMessage, TransactionStatus,
    },
    middleware::{MessageMiddleware, Middleware},
    pipeline::{FrontendHandling, OperationId},
    transport::Buffered,
};
use serde::Serialize;
use sqltk::parser::ast::{self, Value};
use sqltk::NodeKey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, info, warn};

async fn receive_frontend<S: AsyncRead + Unpin>(
    reader: &mut Buffered<S, FrontendDirection>,
    connection_timeout: Option<Duration>,
) -> Result<FrontendMessage, Error> {
    match connection_timeout {
        Some(duration) => tokio::time::timeout(duration, reader.receive_wire())
            .await
            .map_err(|_| Error::ConnectionTimeout { duration })?
            .map_err(Into::into),
        None => reader.receive_wire().await.map_err(Into::into),
    }
}

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
pub struct Frontend<R, W, S>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: EncryptionService,
{
    /// Reader for incoming client messages
    client_reader: Buffered<R, FrontendDirection>,
    /// Sender for outgoing messages to client
    client_sender: Sender,
    /// Writer for forwarding messages to server
    server_writer: Buffered<W, BackendDirection>,
    /// Session context tracking statements, portals, and keyset IDs
    context: Context<S>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FrontendDisposition {
    #[default]
    Forward,
    Local,
}

struct FrontendInterceptor<'a, R, W, S>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: EncryptionService,
{
    frontend: &'a mut Frontend<R, W, S>,
}

impl<R, W, S> MessageMiddleware<FrontendMessage, FrontendDisposition>
    for FrontendInterceptor<'_, R, W, S>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: EncryptionService,
{
    type Error = Error;

    async fn intercept(
        &mut self,
        disposition: &mut FrontendDisposition,
        message: FrontendMessage,
    ) -> Result<FrontendMessage, Self::Error> {
        self.frontend.intercept_frontend(disposition, message).await
    }
}

impl<R, W, S> Frontend<R, W, S>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: EncryptionService,
{
    /// Creates a new Frontend instance.
    ///
    /// # Arguments
    ///
    /// * `client_reader` - Stream for reading messages from the PostgreSQL client
    /// * `client_sender` - Channel sender for sending messages back to client
    /// * `server_writer` - Stream for writing messages to the PostgreSQL server
    /// * `context` - Session context for tracking statements and portals with service access
    pub fn new(
        client_reader: R,
        client_sender: Sender,
        server_writer: W,
        context: Context<S>,
    ) -> Self {
        Frontend {
            client_reader: Buffered::new_frontend(client_reader),
            client_sender,
            server_writer: Buffered::new(server_writer),
            context,
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
        let protocol_message =
            receive_frontend(&mut self.client_reader, self.context.connection_timeout()).await?;
        let frame = protocol_message.to_frame()?;
        let sent: u64 = (frame.body.len() + 5) as u64;
        counter!(CLIENTS_BYTES_RECEIVED_TOTAL).increment(sent);

        let tracking_message = protocol_message.clone();
        let (outbound_message, disposition) = {
            let mut middleware = Middleware::new(
                FrontendDisposition::Forward,
                FrontendInterceptor { frontend: self },
            );
            let outbound_message = middleware.intercept(protocol_message).await?;
            (outbound_message, *middleware.state())
        };

        if disposition == FrontendDisposition::Forward {
            self.context
                .protocol_frontend_received(tracking_message, FrontendHandling::Forward)
                .await?;
            self.write_to_server(outbound_message).await?;
        }

        Ok(())
    }

    async fn intercept_frontend(
        &mut self,
        disposition: &mut FrontendDisposition,
        protocol_message: FrontendMessage,
    ) -> Result<FrontendMessage, Error> {
        let mut outbound_message = protocol_message.clone();

        if self.context.mapping_disabled() {
            return Ok(outbound_message);
        }

        let recovering_from_extended_error = self.context.protocol_in_extended_error()?;

        // When an error is detected while processing any extended-query message, the backend issues ErrorResponse, then reads and discards messages until a Sync is reached,
        // https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-EXT-QUERY
        if recovering_from_extended_error {
            warn!(target: PROTOCOL,
                client_id = self.context.client_id,
                message = ?protocol_message,
            );
            if !matches!(protocol_message, FrontendMessage::Sync) {
                self.context
                    .protocol_frontend_received(protocol_message, FrontendHandling::Local)
                    .await?;
                *disposition = FrontendDisposition::Local;
                return Ok(outbound_message);
            }
        }

        let tracking_message = protocol_message.clone();
        match protocol_message {
            FrontendMessage::Query(query) => {
                match self.query_handler(Query::from(query)).await {
                    Ok(Some(mapped)) => outbound_message = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => {
                        warn!(
                            client_id = self.context.client_id,
                            msg = "Query Handler Error",
                            error = ?err.to_string(),
                        );
                        let id = self
                            .context
                            .protocol_frontend_received(tracking_message, FrontendHandling::Local)
                            .await?;
                        self.send_error_response(id, err).await?;
                        self.send_ready_for_query(id).await?;
                        *disposition = FrontendDisposition::Local;
                        return Ok(outbound_message);
                    }
                }
            }
            FrontendMessage::Describe(describe) => {
                self.describe_handler(describe).await?;
            }
            FrontendMessage::Execute(execute) => {
                self.execute_handler(execute).await?;
            }
            FrontendMessage::Parse(parse) => {
                match self.parse_handler(Parse::from(parse)).await {
                    Ok(Some(mapped)) => outbound_message = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => {
                        warn!(
                            client_id = self.context.client_id,
                            msg = "Parse Handler Error",
                            error = ?err.to_string(),
                        );
                        let id = self
                            .context
                            .protocol_frontend_received(tracking_message, FrontendHandling::Local)
                            .await?;
                        self.send_error_response(id, err).await?;
                        *disposition = FrontendDisposition::Local;
                        return Ok(outbound_message);
                    }
                }
            }
            FrontendMessage::Bind(bind) => {
                match self.bind_handler(Bind::try_from(bind)?).await {
                    Ok(Some(mapped)) => outbound_message = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => {
                        let id = self
                            .context
                            .protocol_frontend_received(tracking_message, FrontendHandling::Local)
                            .await?;
                        match err {
                            Error::Mapping(MappingError::InvalidParameter(_)) => {
                                warn!(target: PROTOCOL,
                                    client_id = self.context.client_id,
                                    msg = "EncryptError::InvalidParameter",
                                );
                                self.send_error_response(id, err).await?;
                            }
                            Error::Encrypt(EncryptError::UnknownKeysetIdentifier { .. }) => {
                                warn!(target: PROTOCOL,
                                    client_id = self.context.client_id,
                                    msg = "EncryptError::UnknownKeysetIdentifier",
                                );
                                self.send_error_response(id, err).await?;
                            }
                            _ => {
                                warn!(target: PROTOCOL,
                                    client_id = self.context.client_id,
                                    msg = "Bind Error",
                                    err = err.to_string()
                                );
                                self.send_error_response(id, err).await?;
                            }
                        }
                        *disposition = FrontendDisposition::Local;
                        return Ok(outbound_message);
                    }
                }
            }
            FrontendMessage::Sync => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    message = ?protocol_message,
                );

                self.context.reload_schema_if_changed().await;

                if recovering_from_extended_error {
                    debug!(target: PROTOCOL,
                        client_id = self.context.client_id,
                        msg = "Ready for Query",
                    );
                    let id = self
                        .context
                        .protocol_frontend_received(tracking_message, FrontendHandling::Local)
                        .await?;
                    self.send_ready_for_query(id).await?;
                    *disposition = FrontendDisposition::Local;
                    return Ok(outbound_message);
                }
            }
            FrontendMessage::Close(close) => {
                self.close_handler(close).await?;
            }
            _ => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    msg = "Passthrough",
                    message = ?protocol_message,
                );
            }
        }

        Ok(outbound_message)
    }

    pub async fn write_to_server(&mut self, message: FrontendMessage) -> Result<(), Error> {
        debug!(target: PROTOCOL, msg = "Write to server", ?message);
        let frame = message.to_frame()?;
        let sent: u64 = (frame.body.len() + 5) as u64;
        counter!(SERVER_BYTES_SENT_TOTAL).increment(sent);

        let start = Instant::now();
        self.server_writer.push(frame)?;
        self.server_writer.flush().await?;
        let duration = start.elapsed();
        if let Some(session_id) = self.context.latest_session_id() {
            self.context.add_server_write_duration(session_id, duration);
        }

        Ok(())
    }

    pub async fn terminate(&mut self) -> Result<(), Error> {
        debug!(target: PROTOCOL, msg = "Terminate server connection");
        self.write_to_server(FrontendMessage::Terminate).await?;
        Ok(())
    }

    async fn describe_handler(&mut self, describe: Describe) -> Result<(), Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?describe);
        self.context.set_describe(describe);
        Ok(())
    }

    async fn close_handler(&mut self, close: Close) -> Result<(), Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?close);
        match close.target {
            DescribeTarget::Portal => self.context.close_portal(&close.name),
            DescribeTarget::Statement => self.context.close_statement_and_portal(&close.name),
        }
        Ok(())
    }

    async fn execute_handler(&mut self, execute: Execute) -> Result<(), Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?execute);
        self.context
            .set_execute_for_portal(execute.portal.to_owned());
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
    async fn query_handler(&mut self, mut query: Query) -> Result<Option<FrontendMessage>, Error> {
        let handler_start = Instant::now();
        let session_id = self.context.start_session();

        // Set protocol type for diagnostics
        self.context.update_statement_metadata(session_id, |m| {
            m.protocol = Some(ProtocolType::Simple);
        });

        let parse_timer = PhaseTimer::start();

        // Simple Query may contain many statements
        let parsed_statements = SqlParser::parse_statements(&query.statement)?;
        let mut transformed_statements = vec![];

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            statements = parsed_statements.len(),
        );

        let mut portal = Portal::passthrough(Some(session_id));
        let mut encrypted = false;
        let mut parse_duration_recorded = false;

        for statement in &parsed_statements {
            if let Some(mapping_disabled) = self.context.maybe_set_unsafe_disable_mapping(statement)
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

            self.handle_set_keyset(statement)?;

            self.check_for_schema_change(statement);

            if !eql_mapper::requires_type_check(statement) {
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
                continue;
            }

            let typed_statement = match self.type_check(statement) {
                Ok(ts) => ts,
                Err(err) => {
                    // `must_fail_closed` errors are refusals: passing the
                    // original statement to the database is the harm they exist
                    // to prevent, so they ignore the passthrough fallback. This
                    // holds today, independently of CIP-3680 removing that
                    // fallback altogether.
                    if self.context.mapping_errors_enabled() || err.must_fail_closed() {
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
                        // Record parse duration before encryption work starts
                        if !parse_duration_recorded {
                            self.context
                                .record_parse_duration(session_id, parse_timer.elapsed());
                            parse_duration_recorded = true;
                        }

                        let encrypted_literals = self
                            .encrypt_literals(
                                session_id,
                                &typed_statement,
                                &statement.literal_columns,
                            )
                            .await?;

                        if let Some(transformed_statement) = self
                            .transform_statement(&typed_statement, &encrypted_literals)
                            .await?
                        {
                            debug!(target: MAPPER,
                                client_id = self.context.client_id,
                                transformed_statement = ?transformed_statement.statement,
                            );

                            // The simple protocol has no params, so the plan is
                            // always empty here — only the SQL is needed.
                            transformed_statements.push(transformed_statement.statement);
                            encrypted = true;
                        }
                    }

                    counter!(STATEMENTS_ENCRYPTED_TOTAL).increment(1);

                    // Set Encrypted portal and mark as mapped
                    portal = Portal::encrypted(Arc::new(statement), Some(session_id));
                    self.context.update_statement_metadata(session_id, |m| {
                        m.encrypted = true;
                    });
                }
                None => {
                    debug!(target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Passthrough Statement"
                    );
                    counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
                    transformed_statements.push(statement.clone());
                }
            };
        }

        // Record parse/typecheck duration (if not already recorded before encryption)
        if !parse_duration_recorded {
            self.context
                .record_parse_duration(session_id, parse_timer.elapsed());
        }

        // Set statement type based on parsed statements
        let statement_type = if parsed_statements.len() == 1 {
            parsed_statements
                .first()
                .map(StatementType::from_statement)
                .unwrap_or(StatementType::Other)
        } else {
            StatementType::Other
        };
        self.context.update_statement_metadata(session_id, |m| {
            m.statement_type = Some(statement_type);
            m.set_multi_statement(parsed_statements.len() > 1);
        });

        // Set query fingerprint
        self.context.update_statement_metadata(session_id, |m| {
            m.set_query_fingerprint(&query.statement);
        });

        self.context.add_portal(Name::new(), portal);
        self.context.set_execute(Name::new(), Some(session_id));

        if encrypted {
            let transformed_statement = transformed_statements
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(";");

            query.rewrite(transformed_statement.to_string());

            let message = FrontendMessage::from(query);
            let handler_duration = handler_start.elapsed();
            debug!(
                target: MAPPER,
                client_id = self.context.client_id,
                msg = "Rewrite Query",
                transformed_statement = transformed_statement.to_string(),
                duration_ms = handler_duration.as_millis(),
                ?message,
            );
            if handler_duration.as_millis() > 100 {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Slow query handler processing",
                    duration_ms = handler_duration.as_millis(),
                );
            }
            Ok(Some(message))
        } else {
            let handler_duration = handler_start.elapsed();
            if handler_duration.as_millis() > 50 {
                debug!(
                    client_id = self.context.client_id,
                    msg = "Query handler processing time",
                    duration_ms = handler_duration.as_millis(),
                );
            }
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
    /// literals that don't require encryption and `Some(EqlOutput)` for encrypted values.
    async fn encrypt_literals(
        &mut self,
        session_id: SessionId,
        typed_statement: &TypeCheckedStatement<'_>,
        literal_columns: &Vec<Option<Column>>,
    ) -> Result<Vec<Option<EqlOutput>>, Error> {
        let literal_values = typed_statement.literal_values();
        if literal_values.is_empty() {
            debug!(target: MAPPER,
                client_id = self.context.client_id,
                msg = "No literals to encrypt"
            );
            return Ok(vec![]);
        }

        let plaintexts = literals_to_plaintext(typed_statement, literal_columns)?;

        let start = Instant::now();

        let mut encrypted = self
            .context
            .encrypt(plaintexts, literal_columns)
            .await
            .inspect_err(|_| {
                counter!(ENCRYPTION_ERROR_TOTAL).increment(1);
            })?;

        for ((_, literal), encrypted) in literal_values.iter().zip(encrypted.iter_mut()) {
            project_query_operand(
                typed_statement.query_operands.contains_literal(literal),
                encrypted,
            );
        }

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            ?literal_columns,
            ?encrypted
        );

        let duration = Instant::now().duration_since(start);

        // Add to phase timing diagnostics (accumulate)
        self.context.add_encrypt_duration(session_id, duration);

        // Update metadata with encrypted values count
        let encrypted_count = encrypted.iter().filter(|e| e.is_some()).count();
        self.context.update_statement_metadata(session_id, |m| {
            m.encrypted = true;
            m.set_encrypted_values_count(encrypted_count);
        });

        counter!(ENCRYPTION_REQUESTS_TOTAL).increment(1);
        counter!(ENCRYPTED_VALUES_TOTAL).increment(encrypted_count as u64);
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
        encrypted_literals: &Vec<Option<EqlOutput>>,
    ) -> Result<Option<eql_mapper::TransformedStatement>, Error> {
        // Convert literals to ast Expr
        let mut encrypted_expressions = vec![];
        for encrypted in encrypted_literals {
            let e = match encrypted {
                // A JSON selector (RHS of `->`/`->>`, or the `jsonb_path_query`
                // path) is a bare tokenized-selector hash used directly as `text`
                // by the eql_v3 functions (`eql_v3."->"(json, text)`). Bind the raw
                // token: JSON-serializing it (below) would re-quote the bare string
                // (`"<hash>"`), so it would never match the stored per-entry `s`.
                Some(EqlOutput::Query(EqlQueryPayload::Selector(s))) => {
                    Some(Value::SingleQuotedString(s.clone()))
                }
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
            .zip(encrypted_expressions)
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
    async fn parse_handler(
        &mut self,
        mut message: Parse,
    ) -> Result<Option<FrontendMessage>, Error> {
        let session_id = self.context.start_session();

        // Set protocol type
        self.context.update_statement_metadata(session_id, |m| {
            m.protocol = Some(ProtocolType::Extended);
        });

        let parse_timer = PhaseTimer::start();

        self.context
            .set_statement_session(message.name.to_owned(), session_id);

        debug!(
            target: PROTOCOL,
            client_id = self.context.client_id,
            parse = ?message
        );

        // A Parse rebinds the name: whatever it referred to before is gone.
        //
        // Dropping it here rather than only on the mapped path matters, because
        // every path below can return without caching anything — a statement
        // that needs no type check (`BEGIN`, `END`), one parsed while mapping is
        // disabled, or one that fails to type check. Leaving the previous entry
        // in place means the next Bind for this name is rewritten against a
        // statement the client never parsed, and the param counts do not line
        // up:
        //
        //     FATAL: Rewritten statement binds parameter 1, but only 0 were provided
        //
        // pgbench in extended mode is exactly this shape: it reuses the unnamed
        // statement for every command, so the `END` at the close of a
        // transaction binds against the `SELECT` that preceded it.
        //
        // Closing also drops the name's session mapping, so it must happen
        // BEFORE the new session is recorded — the other way around wipes the
        // mapping that was just written and every Bind falls back to the
        // latest-session guess.
        self.context.close_statement(&message.name);
        self.context
            .set_statement_session(message.name.to_owned(), session_id);

        let statement = SqlParser::parse_statement(&message.statement)?;

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
                // See the matching comment on the simple-protocol path: a
                // refusal is never downgraded to a passthrough.
                if self.context.mapping_errors_enabled() || err.must_fail_closed() {
                    return Err(err);
                } else {
                    return Ok(None);
                };
            }
        };

        // Capture the parse message param_types
        // These override the underlying column type
        let param_types = message.param_types.clone();

        let mut parse_duration_recorded = false;

        match self.to_encryptable_statement(&typed_statement, param_types)? {
            Some(mut statement) => {
                if typed_statement.requires_transform() {
                    // Record parse duration before encryption work starts
                    self.context
                        .record_parse_duration(session_id, parse_timer.elapsed());
                    parse_duration_recorded = true;

                    let encrypted_literals = self
                        .encrypt_literals(session_id, &typed_statement, &statement.literal_columns)
                        .await?;

                    if let Some(transformed_statement) = self
                        .transform_statement(&typed_statement, &encrypted_literals)
                        .await?
                    {
                        debug!(target: MAPPER,
                            client_id = self.context.client_id,
                            transformed_statement = ?transformed_statement.statement,
                            param_plan = ?transformed_statement.params,
                        );

                        // The rewrite may have reshaped the params, so the
                        // statement's output params come from the plan rather
                        // than from its own input params.
                        let output_columns = self
                            .context
                            .get_output_param_columns(&transformed_statement.params)?;
                        statement.output_params =
                            output_params_from_plan(&transformed_statement.params, output_columns);

                        message.rewrite_statement(transformed_statement.statement.to_string());
                    }
                }

                counter!(STATEMENTS_ENCRYPTED_TOTAL).increment(1);

                message.rewrite_param_types(&statement.output_params);
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

        // Record parse duration (if not already recorded before encryption)
        if !parse_duration_recorded {
            self.context
                .record_parse_duration(session_id, parse_timer.elapsed());
        }

        // Set statement type and fingerprint
        self.context.update_statement_metadata(session_id, |m| {
            m.statement_type = Some(StatementType::from_statement(&statement));
            m.set_query_fingerprint(&message.statement);
        });

        if message.requires_rewrite() {
            let message = FrontendMessage::from(message);

            debug!(target: MAPPER,
                client_id = self.context.client_id,
                msg = "Rewrite Parse",
                ?message);

            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    ///
    /// Check the Statement AST for DDL
    /// Sets a schema changed flag in the Context
    ///
    ///
    fn check_for_schema_change(&self, statement: &ast::Statement) {
        let schema_changed = eql_mapper::collect_ddl(self.context.get_table_resolver(), statement);

        if schema_changed {
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

            if self.context.default_keyset_id().is_some() {
                debug!(target: MAPPER,
                    client_id = self.context.client_id,
                    default_keyset_id = ?self.context.default_keyset_id(),
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
        let param_columns = self.context.get_param_columns(typed_statement)?;
        let projection_columns = self.context.get_projection_columns(typed_statement)?;
        let literal_columns = self.context.get_literal_columns(typed_statement)?;

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

        // Until the statement is rewritten its output params are its input
        // params — a statement that needs no transform never reshapes them, and
        // one that does overwrites this from the rewrite's `ParamPlan`.
        let output_params = param_columns
            .iter()
            .enumerate()
            .map(|(idx, column)| OutputParam {
                column: column.to_owned(),
                source: OutputParamSource::Input(idx),
                query_operand: false,
            })
            .collect();

        let statement = Statement::new(
            param_columns.to_owned(),
            output_params,
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
    async fn bind_handler(&mut self, mut bind: Bind) -> Result<Option<FrontendMessage>, Error> {
        if self.context.unsafe_disable_mapping() {
            warn!(msg = "Encrypted statement mapping is not enabled");
            counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            return Ok(None);
        }

        let session_id = self
            .context
            .get_statement_session_or_latest(&bind.prepared_statement);

        // Track param bytes for diagnostics
        let param_bytes: usize = bind.param_values.iter().map(|p| p.bytes.len()).sum();
        self.context
            .with_session(session_id, |m| m.metadata.set_param_bytes(param_bytes));

        debug!(target: PROTOCOL, client_id = self.context.client_id, bind = ?bind);

        let mut portal = Portal::passthrough(session_id);

        if let Some(statement) = self.context.get_statement(&bind.prepared_statement) {
            debug!(target:MAPPER, client_id = self.context.client_id, ?statement);

            if statement.has_params() {
                let encrypted = self.encrypt_params(session_id, &bind, &statement).await?;
                bind.rewrite(&statement.output_params, encrypted)?;
            }
            if statement.has_projection() {
                portal = Portal::encrypted_with_format_codes(
                    statement,
                    bind.result_columns_format_codes.to_owned(),
                    session_id,
                );
                self.context
                    .with_session(session_id, |m| m.metadata.encrypted = true);
            }
        };

        debug!(target: MAPPER, client_id = self.context.client_id, portal = ?portal);
        self.context.add_portal(bind.portal.to_owned(), portal);

        if bind.requires_rewrite() {
            let message = FrontendMessage::from(bind);
            debug!(
                target: MAPPER,
                client_id = self.context.client_id,
                msg = "Rewrite Bind",
                ?message
            );

            Ok(Some(message))
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
        session_id: Option<SessionId>,
        bind: &Bind,
        statement: &Statement,
    ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
        let plaintexts =
            bind.to_plaintext(&statement.output_params, &statement.postgres_param_types)?;

        // Encryption is positional over the OUTPUT params — the values actually
        // sent — not over what the client bound.
        let output_param_columns = statement
            .output_params
            .iter()
            .map(|output| output.column.to_owned())
            .collect::<Vec<_>>();

        debug!(target: MAPPER, client_id = self.context.client_id, plaintexts = ?plaintexts);

        let start = Instant::now();

        let mut encrypted = self
            .context
            .encrypt(plaintexts, &output_param_columns)
            .await
            .inspect_err(|_| {
                counter!(ENCRYPTION_ERROR_TOTAL).increment(1);
            })?;

        for (output, encrypted) in statement.output_params.iter().zip(encrypted.iter_mut()) {
            project_query_operand(output.query_operand, encrypted);
        }

        let duration = Instant::now().duration_since(start);

        // Record timing and metadata for this encryption operation
        let encrypted_count = encrypted.iter().filter(|e| e.is_some()).count();
        self.context.with_session(session_id, |m| {
            // Add to phase timing diagnostics (accumulate)
            m.phase_timing.add_encrypt(duration);
            // Always update metadata for slow-statement logging
            m.metadata.encrypted = true;
            m.metadata.set_encrypted_values_count(encrypted_count);
        });

        // Prometheus metrics remain gated
        if self.context.prometheus_enabled() {
            counter!(ENCRYPTION_REQUESTS_TOTAL).increment(1);
            counter!(ENCRYPTED_VALUES_TOTAL).increment(encrypted_count as u64);
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
            // A column this build cannot map is a refusal, not a coverage gap,
            // so it gets its own error: `must_fail_closed` keys off it to stop
            // the passthrough fallback from serving the statement anyway.
            Err(err) if err.as_unmappable_encrypted_column().is_some() => {
                let (table, column, column_type) = err.as_unmappable_encrypted_column().unwrap();
                warn!(
                    client_id = self.context.client_id,
                    msg = "Statement refused: it references a column declared with a legacy EQL v2 type that this build cannot encrypt or decrypt. Migrate the column to an EQL v3 domain type.",
                    table,
                    column,
                    column_type,
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::UnmappableEncryptedColumn {
                    table: table.to_string(),
                    column: column.to_string(),
                    column_type: column_type.to_string(),
                }
                .into())
            }
            Err(EqlMapperError::InternalError(str)) => {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Internal Error in EQL Mapper",
                    mapping_errors_enabled = self.context.mapping_errors_enabled(),
                    error = str,
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::Internal(str).into())
            }
            Err(err) => {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Unmappable statement",
                    mapping_errors_enabled = self.context.mapping_errors_enabled(),
                    error = err.to_string(),
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::StatementCouldNotBeTypeChecked(err.to_string()).into())
            }
        }
    }

    ///
    /// Send a ReadyForQuery to the client, answering a Sync (or simple Query)
    /// for a batch of which nothing reached the server.
    ///
    async fn send_ready_for_query(&mut self, id: OperationId) -> Result<(), Error> {
        let message = BackendMessage::ReadyForQuery(TransactionStatus::Idle);

        debug!(target: PROTOCOL,
            client_id = self.context.client_id,
            msg = "send_ready_for_query",
            ?message,
        );

        self.context
            .protocol_backend_local(id, message.clone())
            .await?;
        self.client_sender.send(message)?;
        self.context.protocol_backend_sent();
        Ok(())
    }
}

/// Projects a stored payload into its query operand when the value is bound in
/// a predicate rather than stored.
///
/// A query operand carries the column's search terms but never a decryptable
/// ciphertext — the `eql_v3.query_*` domains reject `c` outright. It cannot be
/// produced by encrypting differently: a single `EqlOperation::Query` yields
/// only ONE term, while an operand may need several (`{v,i,hm,ob}`), so the
/// value is encrypted in Store mode and projected here.
///
/// Terms that are already query-shaped (the JSON selectors and SteVec terms,
/// which the encryptor produces via `EqlOperation::Query`) are left alone.
fn project_query_operand(query_operand: bool, encrypted: &mut Option<EqlOutput>) {
    if !query_operand {
        return;
    }

    match encrypted.take() {
        Some(EqlOutput::Store(ciphertext)) => {
            *encrypted = Some(EqlOutput::Query(ciphertext.into_query_operand()));
        }
        already_query_shaped => *encrypted = already_query_shaped,
    }
}

fn literals_to_plaintext(
    typed_statement: &TypeCheckedStatement<'_>,
    literal_columns: &Vec<Option<Column>>,
) -> Result<Vec<Option<Plaintext>>, Error> {
    let literals = typed_statement.literal_values();

    let plaintexts = literals
        .iter()
        .zip(literal_columns)
        .map(|((eql_term, val), col)| match col {
            Some(col) => {
                let plaintext = match eql_term.variant() {
                    EqlTermVariant::JsonValueSelector => {
                        json_value_selector_literal_plaintext(typed_statement, val)
                    }
                    // A selector that carries a collapsed chain keys the composed
                    // path, not the one segment it spells. Only a selector the
                    // mapper recorded a chain for: a single access has no record
                    // and takes the ordinary single-segment route below.
                    EqlTermVariant::JsonAccessor
                        if typed_statement
                            .json_accessor_paths
                            .for_literal(val)
                            .is_some() =>
                    {
                        json_accessor_path_literal_plaintext(typed_statement, val)
                    }
                    _ => literal_from_sql(val, col.eql_term(), col.cast_type()),
                };

                plaintext.map_err(|err| {
                    debug!(
                        target: MAPPER,
                        msg = "Could not convert literal value",
                        value = ?val,
                        cast_type = ?col.cast_type(),
                        error = err.to_string()
                    );
                    MappingError::InvalidParameter(Box::new(col.to_owned())).into()
                })
            }
            None => Ok(None),
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(plaintexts)
}

/// Composes the needle for a JSON field equality whose value is a literal:
/// `{"path": <selector>, "value": <literal>}`.
///
/// Only a literal path can be resolved here — the whole statement is encrypted
/// at Parse time, before any param is bound. `col -> $1 = 'value'` (param path,
/// literal value) is therefore not supported; it is also not a shape any client
/// produces, since a client that parameterises the path parameterises the value
/// too.
fn json_value_selector_literal_plaintext(
    typed_statement: &TypeCheckedStatement<'_>,
    literal: &ast::Value,
) -> Result<Option<Plaintext>, MappingError> {
    let path: Option<Vec<&str>> = typed_statement
        .json_value_selectors
        .for_literal(literal)
        .and_then(|source| {
            source
                .segments()
                .iter()
                .map(|segment| match segment {
                    JsonSelectorSegment::Literal(selector) => Some(selector.as_str()),
                    JsonSelectorSegment::Param(_) => None,
                })
                .collect::<Option<Vec<_>>>()
        });

    let Some(path) = path else {
        debug!(
            target: MAPPER,
            msg = "Encrypted JSON equality needs a literal selector when the value is a literal",
            value = ?literal,
        );
        return Err(MappingError::CouldNotParseParameter);
    };

    let Some(value) = literal_json_value(literal)? else {
        return Ok(None);
    };

    json_value_selector_plaintext(&path, value).map(Some)
}

/// Composes the eJSONPath for the selector of a collapsed accessor chain whose
/// steps are all literals: `j -> 'a' -> 'b'` keys `$.a.b`.
///
/// Only an all-literal path can be resolved here — the whole statement is
/// encrypted at Parse time, before any param is bound. `j -> $1 -> 'b'` (param
/// step, literal outermost selector) is therefore not supported: the surviving
/// operand is the literal, which must be encrypted now, while the step in front
/// of it is not known until Bind. The mirror image, `j -> 'a' -> $1`, works — the
/// surviving operand is the param, so the whole path resolves at Bind.
///
/// Refusing is the only safe answer. Composing what is known would key `$.b` and
/// read a different field, silently.
fn json_accessor_path_literal_plaintext(
    typed_statement: &TypeCheckedStatement<'_>,
    literal: &ast::Value,
) -> Result<Option<Plaintext>, MappingError> {
    let path: Option<Vec<&str>> = typed_statement
        .json_accessor_paths
        .for_literal(literal)
        .and_then(|source| {
            source
                .segments()
                .iter()
                .map(|segment| match segment {
                    JsonSelectorSegment::Literal(selector) => Some(selector.as_str()),
                    JsonSelectorSegment::Param(_) => None,
                })
                .collect::<Option<Vec<_>>>()
        });

    let Some(path) = path else {
        debug!(
            target: MAPPER,
            msg = "An encrypted JSON path with a placeholder step must end in a placeholder, \
                   so that the whole path can be resolved when the params are bound",
            value = ?literal,
        );
        return Err(MappingError::CouldNotParseParameter);
    };

    Ok(Some(Plaintext::new(compose_json_selector_path(&path))))
}

fn to_json_literal_value<T>(literal: &T) -> Result<Value, Error>
where
    T: ?Sized + Serialize,
{
    Ok(serde_json::to_string(literal).map(Value::SingleQuotedString)?)
}

/// Implementation of PostgreSQL error handling for the Frontend component.
impl<R, W, S> PostgreSqlErrorHandler for Frontend<R, W, S>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: EncryptionService,
{
    fn client_sender(&mut self) -> &mut Sender {
        &mut self.client_sender
    }

    fn client_id(&self) -> i32 {
        self.context.client_id
    }
}

impl<R, W, S> Frontend<R, W, S>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: EncryptionService,
{
    async fn send_error_response(&mut self, id: OperationId, err: Error) -> Result<(), Error> {
        let error_response = self.error_to_response(err);
        let message = error_response.into_backend_message();

        debug!(target: PROTOCOL,
            client_id = self.context.client_id,
            msg = "send_error_response",
            ?message,
        );

        self.context
            .protocol_backend_local(id, message.clone())
            .await?;
        self.client_sender.send(message)?;
        self.context.protocol_backend_sent();
        Ok(())
    }
}
