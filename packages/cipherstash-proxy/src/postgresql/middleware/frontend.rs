use super::super::context::phase_timing::PhaseTimer;
use super::super::context::{Context, SessionId, Statement};
use super::super::error_handler::PostgreSqlErrorHandler;
use super::super::parser::SqlParser;
use super::super::rewrite::bind::Bind;
use crate::error::{EncryptError, Error, MappingError};
use crate::log::{ENCRYPT, MAPPER, PROTOCOL};
use crate::postgresql::context::column::Column;
use crate::postgresql::context::statement::{
    output_params_from_plan, OutputParam, OutputParamSource,
};
use crate::postgresql::context::statement_metadata::{ProtocolType, StatementType};
use crate::postgresql::context::Portal;
use crate::postgresql::data::{
    compose_json_selector_path, json_value_selector_plaintext, literal_from_sql, literal_json_value,
};
use crate::postgresql::inbound_eql;
use crate::postgresql::rewrite::Name;
use crate::postgresql::rewrite::UNSPECIFIED_TYPE_OID;
use crate::postgresql::OperationId;
use crate::prometheus::{
    ENCRYPTED_VALUES_TOTAL, ENCRYPTION_DURATION_SECONDS, ENCRYPTION_ERROR_TOTAL,
    ENCRYPTION_REQUESTS_TOTAL, STATEMENTS_ENCRYPTED_TOTAL,
    STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL, STATEMENTS_PASSTHROUGH_TOTAL,
    STATEMENTS_UNMAPPABLE_TOTAL,
};
use crate::proxy::EncryptionService;
use crate::{EqlOutput, EqlQueryPayload};
use cipherstash_client::encryption::Plaintext;
use eql_mapper::{self, EqlMapperError, EqlTermVariant, JsonSelectorSegment, TypeCheckedStatement};
use metrics::{counter, histogram};
use pg_proto::{
    Close, Describe, DescribeTarget, Execute, FrontendMessage, FrontendMiddlewareOutput, Parse,
};
use serde::Serialize;
use sqltk::parser::ast::{self, Value};
use sqltk::NodeKey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

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
/// - **Context Management**: Track statements, portals, and connection state
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
pub struct Frontend<S: EncryptionService> {
    /// Session context tracking statements, portals, and keyset IDs
    context: Context<S>,
    failed_extended_batch: bool,
}

impl<S: EncryptionService> Frontend<S> {
    /// Creates a new Frontend instance.
    ///
    /// # Arguments
    ///
    /// * `client_reader` - Stream for reading messages from the PostgreSQL client
    /// * `client_sender` - Channel sender for sending messages back to client
    /// * `server_writer` - Stream for writing messages to the PostgreSQL server
    /// * `context` - Session context for tracking statements and portals with service access
    pub fn new(context: Context<S>) -> Self {
        Frontend {
            context,
            failed_extended_batch: false,
        }
    }

    pub async fn intercept(
        &mut self,
        operation: OperationId,
        protocol_message: FrontendMessage,
    ) -> Result<FrontendMiddlewareOutput, Error> {
        if matches!(
            protocol_message,
            FrontendMessage::Parse(_) | FrontendMessage::Bind(_)
        ) {
            self.context.set_non_execution(operation)?;
        }
        if self.context.mapping_disabled() {
            match &protocol_message {
                FrontendMessage::Query(_) => {
                    let metrics_scope = self.context.start_metrics_scope()?;
                    self.context.add_portal(
                        operation,
                        Name::new(),
                        Portal::passthrough(Some(metrics_scope)),
                    )?;
                    self.context.set_simple_query_execute_until_ready(
                        operation,
                        Name::new(),
                        Some(metrics_scope),
                    )?;
                }
                FrontendMessage::Bind(bind) => {
                    let session_id = self.context.get_statement_metrics_scope(&bind.statement)?;
                    self.context.add_portal(
                        operation,
                        bind.portal.clone(),
                        Portal::passthrough(session_id),
                    )?;
                }
                FrontendMessage::Execute(execute) => self
                    .context
                    .set_execute_for_portal(operation, execute.portal.clone())?,
                FrontendMessage::Describe(describe) => {
                    self.context.set_describe(operation, describe.clone())?;
                }
                FrontendMessage::Close(close) => match close.target {
                    DescribeTarget::Portal => self.context.close_portal(&close.name)?,
                    DescribeTarget::Statement => {
                        self.context.close_statement_explicit(&close.name)?;
                    }
                },
                FrontendMessage::Parse(parse) => {
                    self.context.close_statement(&parse.statement)?;
                }
                _ => {}
            }
            return Ok(FrontendMiddlewareOutput::Forward(protocol_message));
        }

        if self.failed_extended_batch {
            if matches!(protocol_message, FrontendMessage::Sync) {
                self.failed_extended_batch = false;
                self.context.mark_schema_protocol_boundary();
                return Ok(FrontendMiddlewareOutput::Forward(protocol_message));
            }
            self.context.discard_operation(operation)?;
            return Ok(FrontendMiddlewareOutput::Suppress(protocol_message));
        }

        let mut outbound_message = protocol_message.clone();
        let mut flush_after_forward = false;

        match protocol_message {
            FrontendMessage::Query(query) => {
                let session_id = self.context.start_metrics_scope()?;
                match self.query_handler(operation, query, session_id).await {
                    Ok(Some(mapped)) => outbound_message = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => {
                        self.context.finish_metrics_scope(Some(session_id))?;
                        warn!(
                            client_id = self.context.client_id,
                            msg = "Query Handler Error",
                            error = ?err.to_string(),
                        );
                        let response = self.error_to_response(err);
                        self.context
                            .set_operation_error(operation, response.clone())?;
                        self.context.set_execute(operation, Name::new(), None)?;
                        self.context.mark_schema_protocol_boundary();
                        outbound_message = self.simple_error_query(&response);
                    }
                }
            }
            FrontendMessage::Describe(describe) => {
                self.describe_handler(operation, describe).await?;
            }
            FrontendMessage::Execute(execute) => {
                flush_after_forward = self.execute_handler(operation, execute).await?;
            }
            FrontendMessage::Parse(parse) => {
                let statement = parse.statement.clone();
                let failed_parse = parse.clone();
                match self.parse_handler(parse).await {
                    Ok(Some(mapped)) => outbound_message = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => {
                        self.context.close_statement(&statement)?;
                        warn!(
                            client_id = self.context.client_id,
                            msg = "Parse Handler Error",
                            error = ?err.to_string(),
                        );
                        let response = self.error_to_response(err);
                        self.context
                            .set_operation_error(operation, response.clone())?;
                        self.failed_extended_batch = true;
                        outbound_message = self.extended_parse_error(failed_parse, &response);
                    }
                }
            }
            FrontendMessage::Bind(bind) => {
                let failed_bind = bind.clone();
                match self.bind_handler(operation, Bind::try_from(bind)?).await {
                    Ok(Some(mapped)) => outbound_message = mapped,
                    // No mapping needed, don't change the bytes
                    Ok(None) => (),
                    Err(err) => match err {
                        Error::Mapping(MappingError::InvalidParameter(_)) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "EncryptError::InvalidParameter",
                            );
                            let response = self.error_to_response(err);
                            self.context
                                .set_operation_error(operation, response.clone())?;
                            self.failed_extended_batch = true;
                            outbound_message = self.extended_bind_error(failed_bind, &response);
                        }
                        Error::Encrypt(EncryptError::UnknownKeysetIdentifier { .. }) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "EncryptError::UnknownKeysetIdentifier",
                            );
                            let response = self.error_to_response(err);
                            self.context
                                .set_operation_error(operation, response.clone())?;
                            self.failed_extended_batch = true;
                            outbound_message = self.extended_bind_error(failed_bind, &response);
                        }
                        _ => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "Bind Error",
                                err = err.to_string()
                            );
                            let response = self.error_to_response(err);
                            self.context
                                .set_operation_error(operation, response.clone())?;
                            self.failed_extended_batch = true;
                            outbound_message = self.extended_bind_error(failed_bind, &response);
                        }
                    },
                }
            }
            FrontendMessage::Sync => {
                debug!(target: PROTOCOL,
                    client_id = self.context.client_id,
                    message = ?protocol_message,
                );
                self.context.mark_schema_protocol_boundary();
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

        if flush_after_forward {
            Ok(FrontendMiddlewareOutput::ForwardThenFlush(outbound_message))
        } else {
            Ok(FrontendMiddlewareOutput::Forward(outbound_message))
        }
    }

    async fn describe_handler(
        &mut self,
        operation: OperationId,
        describe: Describe,
    ) -> Result<(), Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?describe);
        self.context.set_describe(operation, describe)?;
        Ok(())
    }

    async fn close_handler(&mut self, close: Close) -> Result<(), Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?close);
        match close.target {
            DescribeTarget::Portal => self.context.close_portal(&close.name)?,
            DescribeTarget::Statement => {
                self.context.close_statement_explicit(&close.name)?;
            }
        }
        Ok(())
    }

    async fn execute_handler(
        &mut self,
        operation: OperationId,
        execute: Execute,
    ) -> Result<bool, Error> {
        debug!(target: PROTOCOL, client_id = self.context.client_id, ?execute);
        let executes_ddl = self.context.execute_schema_portal(&execute.portal);
        self.context
            .set_execute_for_portal(operation, execute.portal.to_owned())?;
        Ok(executes_ddl)
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
    async fn query_handler(
        &mut self,
        operation: OperationId,
        query: bytes::Bytes,
        session_id: SessionId,
    ) -> Result<Option<FrontendMessage>, Error> {
        let handler_start = Instant::now();

        // Set protocol type for diagnostics
        self.context.update_statement_metadata(session_id, |m| {
            m.protocol = Some(ProtocolType::Simple);
        })?;

        let parse_timer = PhaseTimer::start();

        // Simple Query may contain many statements
        let query_text = String::from_utf8_lossy(&query).into_owned();
        let parsed_statements = SqlParser::parse_statements(&query_text)?;
        self.context.prepare_schema_for_statement().await?;
        if self
            .context
            .simple_query_requires_fail_closed(&parsed_statements)
        {
            return Err(MappingError::DependentStatementAfterDdl.into());
        }
        if parsed_statements
            .iter()
            .any(eql_mapper::requires_type_check)
        {
            self.context.wait_for_schema_execution().await;
            self.context.ensure_schema_modelled()?;
        }
        let mut forwarded_statements = vec![];

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
                forwarded_statements.push(statement.clone());
                continue;
            }

            self.handle_set_keyset(statement)?;

            if !eql_mapper::requires_type_check(statement) {
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
                forwarded_statements.push(statement.clone());
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
                        self.record_simple_schema_execution(
                            operation,
                            session_id,
                            Portal::passthrough(Some(session_id)),
                            &parsed_statements,
                        )?;
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

                    let mut transformed = false;
                    if typed_statement.requires_transform() {
                        // Record parse duration before encryption work starts
                        if !parse_duration_recorded {
                            self.context
                                .record_parse_duration(session_id, parse_timer.elapsed())?;
                            parse_duration_recorded = true;
                        }

                        let encrypted_literals = self
                            .encrypt_literals(
                                session_id,
                                &typed_statement,
                                &statement.literal_columns,
                            )
                            .await?;

                        if let Some(transformed_statement) =
                            self.transform_statement(&typed_statement, &encrypted_literals)?
                        {
                            debug!(target: MAPPER,
                                client_id = self.context.client_id,
                                transformed_statement = ?transformed_statement.statement,
                            );

                            // The simple protocol has no params, so the plan is
                            // always empty here — only the SQL is needed.
                            forwarded_statements.push(transformed_statement.statement);
                            encrypted = true;
                            transformed = true;
                        }
                    }

                    if !transformed {
                        forwarded_statements.push(typed_statement.statement.clone());
                    }

                    counter!(STATEMENTS_ENCRYPTED_TOTAL).increment(1);

                    // Set Encrypted portal and mark as mapped
                    portal = Portal::encrypted(Arc::new(statement), Some(session_id));
                    self.context.update_statement_metadata(session_id, |m| {
                        m.encrypted = true;
                    })?;
                }
                None => {
                    debug!(target: MAPPER,
                        client_id = self.context.client_id,
                        msg = "Passthrough Statement"
                    );
                    counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
                    forwarded_statements.push(statement.clone());
                }
            };
        }

        // Record parse/typecheck duration (if not already recorded before encryption)
        if !parse_duration_recorded {
            self.context
                .record_parse_duration(session_id, parse_timer.elapsed())?;
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
        })?;

        // Set query fingerprint
        self.context.update_statement_metadata(session_id, |m| {
            m.set_query_fingerprint(&query_text);
        })?;

        self.record_simple_schema_execution(operation, session_id, portal, &forwarded_statements)?;

        if encrypted {
            let transformed_statement = forwarded_statements
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(";");

            let message = FrontendMessage::Query(bytes::Bytes::from(transformed_statement.clone()));
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

    /// Records the portal and schema intents for the statements actually sent to PostgreSQL.
    fn record_simple_schema_execution(
        &mut self,
        operation: OperationId,
        session_id: SessionId,
        portal: Portal,
        statements: &[ast::Statement],
    ) -> Result<(), Error> {
        self.context.add_portal(operation, Name::new(), portal)?;
        self.context.set_simple_query_execute_until_ready(
            operation,
            Name::new(),
            Some(session_id),
        )?;
        self.context.execute_simple_schema_statements(statements);
        Ok(())
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

        let inbound = literal_values
            .iter()
            .zip(literal_columns)
            .map(|((_, literal), column)| {
                let (Some(column), Some(value)) = (column, (*literal).clone().into_string()) else {
                    return Ok(None);
                };
                inbound_eql::parse(
                    value.as_bytes(),
                    column,
                    literal_is_query_operand(typed_statement, literal, column),
                )
                .map_err(Error::from)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let skip = inbound.iter().map(Option::is_some).collect::<Vec<_>>();
        let plaintexts = literals_to_plaintext_skipping(typed_statement, literal_columns, &skip)?;

        let start = Instant::now();

        let mut encrypted = self
            .context
            .encrypt(plaintexts, literal_columns)
            .await
            .inspect_err(|_| {
                counter!(ENCRYPTION_ERROR_TOTAL).increment(1);
            })?;

        self.merge_inbound_eql(&mut encrypted, inbound, literal_columns)
            .await?;

        for (((_, literal), column), encrypted) in literal_values
            .iter()
            .zip(literal_columns)
            .zip(encrypted.iter_mut())
        {
            let query_operand = column
                .as_ref()
                .is_some_and(|column| literal_is_query_operand(typed_statement, literal, column));
            project_query_operand(query_operand, encrypted);
        }

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            ?literal_columns,
            ?encrypted
        );

        let duration = Instant::now().duration_since(start);

        // Add to phase timing diagnostics (accumulate)
        self.context.add_encrypt_duration(session_id, duration)?;

        // Update metadata with encrypted values count
        let encrypted_count = encrypted.iter().filter(|e| e.is_some()).count();
        self.context.update_statement_metadata(session_id, |m| {
            m.encrypted = true;
            m.set_encrypted_values_count(encrypted_count);
        })?;

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
    fn transform_statement(
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
        let original_query = message.query.clone();
        let session_id = self.context.start_metrics_scope()?;

        // Set protocol type
        self.context.update_statement_metadata(session_id, |m| {
            m.protocol = Some(ProtocolType::Extended);
        })?;

        let parse_timer = PhaseTimer::start();

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
        // Closing also drops the name's metrics scope mapping, so it must happen
        // BEFORE the new scope is recorded — the other way around wipes the
        // mapping that was just written and every Bind falls back to the
        // latest-scope guess.
        self.context.close_statement(&message.statement)?;
        self.context
            .set_statement_metrics_scope(message.statement.to_owned(), session_id)?;

        let mut statement_text = String::from_utf8_lossy(&message.query).into_owned();
        let statement = SqlParser::parse_statement(&statement_text)?;

        self.context.prepare_schema_for_statement().await?;
        if eql_mapper::requires_type_check(&statement) {
            self.context.wait_for_schema_execution().await;
            self.context.ensure_schema_modelled()?;
        }
        self.context
            .prepare_schema_statement(message.statement.to_owned(), statement.clone());

        // Record diagnostics before any passthrough path can return early.
        // Statements that do not require EQL type checking (for example,
        // plaintext INSERTs and SELECT pg_sleep(...)) still need their real
        // statement type and fingerprint in metrics.
        self.context.update_statement_metadata(session_id, |m| {
            m.statement_type = Some(StatementType::from_statement(&statement));
            m.set_query_fingerprint(&statement_text);
        })?;

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
        let client_param_types = message.parameter_types.clone();
        let param_types = client_param_types.iter().map(|oid| *oid as i32).collect();

        let mut parse_duration_recorded = false;

        match self.to_encryptable_statement(&typed_statement, param_types)? {
            Some(mut statement) => {
                if typed_statement.requires_transform() {
                    // Record parse duration before encryption work starts
                    self.context
                        .record_parse_duration(session_id, parse_timer.elapsed())?;
                    parse_duration_recorded = true;

                    let encrypted_literals = self
                        .encrypt_literals(session_id, &typed_statement, &statement.literal_columns)
                        .await?;

                    if let Some(transformed_statement) =
                        self.transform_statement(&typed_statement, &encrypted_literals)?
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

                        statement_text = transformed_statement.statement.to_string();
                        message.query = bytes::Bytes::copy_from_slice(statement_text.as_bytes());
                    }
                }

                counter!(STATEMENTS_ENCRYPTED_TOTAL).increment(1);

                message.parameter_types =
                    rewrite_parse_param_types(&client_param_types, &statement.output_params);
                self.context
                    .add_statement(message.statement.to_owned(), statement)?;
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
                .record_parse_duration(session_id, parse_timer.elapsed())?;
        }

        if message.query != original_query || message.parameter_types != client_param_types {
            let message = FrontendMessage::Parse(message);

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
    async fn bind_handler(
        &mut self,
        operation: OperationId,
        mut bind: Bind,
    ) -> Result<Option<FrontendMessage>, Error> {
        self.context
            .bind_schema_statement(bind.portal.to_owned(), &bind.prepared_statement);
        if self.context.unsafe_disable_mapping() {
            warn!(msg = "Encrypted statement mapping is not enabled");
            counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            self.context.add_portal(
                operation,
                bind.portal.to_owned(),
                Portal::passthrough(None),
            )?;
            return Ok(None);
        }

        let session_id = self
            .context
            .start_portal_metrics_scope(&bind.prepared_statement)?;

        let result = async {
            // Track param bytes for diagnostics
            let param_bytes: usize = bind.param_values.iter().map(|p| p.bytes.len()).sum();
            self.context
                .with_metrics_scope(session_id, |m| m.metadata.set_param_bytes(param_bytes))?;

            debug!(target: PROTOCOL, client_id = self.context.client_id, bind = ?bind);

            let mut portal = Portal::passthrough(session_id);

            if let Some(statement) = self.context.get_statement(&bind.prepared_statement)? {
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
                        .with_metrics_scope(session_id, |m| m.metadata.encrypted = true)?;
                }
            };

            debug!(target: MAPPER, client_id = self.context.client_id, portal = ?portal);
            self.context
                .add_portal(operation, bind.portal.to_owned(), portal)?;

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
        .await;

        if result.is_err() {
            self.context.finish_metrics_scope(session_id)?;
        }

        result
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
        let inbound = bind.inbound_eql(&statement.output_params)?;
        let skip = inbound.iter().map(Option::is_some).collect::<Vec<_>>();
        let plaintexts = bind.to_plaintext_skipping(
            &statement.output_params,
            &statement.postgres_param_types,
            &skip,
        )?;

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

        self.merge_inbound_eql(&mut encrypted, inbound, &output_param_columns)
            .await?;

        for (output, encrypted) in statement.output_params.iter().zip(encrypted.iter_mut()) {
            project_query_operand(output.query_operand, encrypted);
        }

        let duration = Instant::now().duration_since(start);

        // Record timing and metadata for this encryption operation
        let encrypted_count = encrypted.iter().filter(|e| e.is_some()).count();
        self.context.with_metrics_scope(session_id, |m| {
            // Add to phase timing diagnostics (accumulate)
            m.phase_timing.add_encrypt(duration);
            // Always update metadata for slow-statement logging
            m.metadata.encrypted = true;
            m.metadata.set_encrypted_values_count(encrypted_count);
        })?;

        // Prometheus metrics remain gated
        if self.context.prometheus_enabled() {
            counter!(ENCRYPTION_REQUESTS_TOTAL).increment(1);
            counter!(ENCRYPTED_VALUES_TOTAL).increment(encrypted_count as u64);
            histogram!(ENCRYPTION_DURATION_SECONDS).record(duration);
        }

        Ok(encrypted)
    }

    /// Merge application-generated EQL values into the encryption output.
    /// Stored payloads are authenticated and independently verified. Query-only
    /// payloads have no ciphertext to authenticate and are accepted only after
    /// role-aware structural validation in `inbound_eql::parse`.
    async fn merge_inbound_eql(
        &self,
        encrypted: &mut [Option<EqlOutput>],
        inbound: Vec<Option<inbound_eql::InboundEql>>,
        columns: &[Option<Column>],
    ) -> Result<(), Error> {
        let mut positions = Vec::new();
        for (index, (payload, column)) in inbound.into_iter().zip(columns).enumerate() {
            match payload {
                Some(inbound_eql::InboundEql::Query(query)) => {
                    encrypted[index] = Some(EqlOutput::Query(query));
                }
                Some(inbound_eql::InboundEql::Store(ciphertext)) => {
                    let Some(column) = column else {
                        return Err(EncryptError::InvalidInboundEqlPayload.into());
                    };
                    positions.push((index, ciphertext, column.clone()));
                }
                None => {}
            }
        }
        if positions.is_empty() {
            return Ok(());
        }

        let ciphertexts = positions
            .iter()
            .map(|(_, ciphertext, _)| Some(ciphertext.clone()))
            .collect();
        let plaintexts = self
            .context
            .decrypt_inbound_eql(ciphertexts)
            .await
            .map_err(|err| {
                warn!(
                    target: ENCRYPT,
                    client_id = self.context.client_id,
                    msg = "Inbound EQL ciphertext authentication failed",
                    error = ?err,
                );
                EncryptError::InvalidInboundEqlPayload
            })?;

        // Re-encrypt the authenticated plaintext for the inferred destination
        // and compare every derived SEM term. This detects term splicing: the
        // AEAD tag authenticates `c`, but the searchable metadata sits outside
        // it in the EQL envelope.
        let verification_columns = positions
            .iter()
            .map(|(_, _, column)| Some(column.clone()))
            .collect::<Vec<_>>();
        let derived = self
            .context
            .encrypt(plaintexts, &verification_columns)
            .await
            .map_err(|err| {
                warn!(
                    target: ENCRYPT,
                    client_id = self.context.client_id,
                    msg = "Inbound EQL ciphertext metadata verification failed",
                    error = ?err,
                );
                EncryptError::InvalidInboundEqlPayload
            })?;

        for ((index, ciphertext, _), derived) in positions.into_iter().zip(derived) {
            let Some(EqlOutput::Store(derived)) = derived else {
                warn!(
                    target: ENCRYPT,
                    client_id = self.context.client_id,
                    msg = "Inbound EQL ciphertext SEM terms did not match plaintext",
                );
                return Err(EncryptError::InvalidInboundEqlPayload.into());
            };
            if !inbound_eql::sem_terms_match(&ciphertext, derived) {
                warn!(
                    target: ENCRYPT,
                    client_id = self.context.client_id,
                    msg = "Inbound EQL ciphertext SEM terms did not match plaintext",
                );
                return Err(EncryptError::InvalidInboundEqlPayload.into());
            }
            encrypted[index] = Some(EqlOutput::Store(ciphertext));
        }
        Ok(())
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
}

fn quote_literal(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "''");
    if value.contains('\\') {
        format!("E'{escaped}'")
    } else {
        format!("'{escaped}'")
    }
}

fn rewrite_parse_param_types(client_types: &[u32], output_params: &[OutputParam]) -> Vec<u32> {
    if client_types.is_empty() {
        return Vec::new();
    }

    output_params
        .iter()
        .map(|output| match &output.column {
            Some(column) => match column.eql_term {
                EqlTermVariant::JsonAccessor | EqlTermVariant::JsonPath => {
                    postgres_types::Type::TEXT.oid()
                }
                _ => postgres_types::Type::JSONB.oid(),
            },
            None => client_types
                .get(output.source.primary_input())
                .copied()
                .unwrap_or(UNSPECIFIED_TYPE_OID as u32),
        })
        .collect()
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

/// JSON path/accessor literals are query operands even though they are passed
/// as bare text and therefore need no query-domain cast. All other literals
/// use the predicate roles recorded by EQL Mapper.
fn literal_is_query_operand(
    typed_statement: &TypeCheckedStatement<'_>,
    literal: &ast::Value,
    column: &Column,
) -> bool {
    matches!(
        column.eql_term,
        EqlTermVariant::JsonAccessor | EqlTermVariant::JsonPath
    ) || typed_statement.query_operands.contains_literal(literal)
}

fn literals_to_plaintext_skipping(
    typed_statement: &TypeCheckedStatement<'_>,
    literal_columns: &Vec<Option<Column>>,
    skip: &[bool],
) -> Result<Vec<Option<Plaintext>>, Error> {
    let literals = typed_statement.literal_values();

    let plaintexts = literals
        .iter()
        .zip(literal_columns)
        .enumerate()
        .map(|(index, ((eql_term, val), col))| {
            if skip.get(index).copied().unwrap_or(false) {
                return Ok(None);
            }
            match col {
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
            }
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
impl<S: EncryptionService> PostgreSqlErrorHandler for Frontend<S> {
    fn client_id(&self) -> i32 {
        self.context.client_id
    }
}

impl<S: EncryptionService> Frontend<S> {
    fn simple_error_query(&self, response: &pg_proto::DiagnosticResponse) -> FrontendMessage {
        let options = response
            .fields
            .iter()
            .filter_map(|field| {
                let option = match field.code {
                    b'M' => "MESSAGE",
                    b'C' => "ERRCODE",
                    b'D' => "DETAIL",
                    b'H' => "HINT",
                    b's' => "SCHEMA",
                    b't' => "TABLE",
                    b'c' => "COLUMN",
                    b'd' => "DATATYPE",
                    b'n' => "CONSTRAINT",
                    _ => return None,
                };
                Some(format!(
                    "{option} = {}",
                    quote_literal(&String::from_utf8_lossy(&field.value))
                ))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let body = format!("BEGIN RAISE EXCEPTION USING {options}; END;");
        FrontendMessage::Query(bytes::Bytes::from(format!("DO {};", quote_literal(&body))))
    }

    fn extended_parse_error(
        &self,
        mut parse: Parse,
        response: &pg_proto::DiagnosticResponse,
    ) -> FrontendMessage {
        let marker = self.extended_error_marker(response);
        parse.query =
            bytes::Bytes::from(format!("SELECT NULL::\"{}\"", marker.replace('"', "\"\"")));
        parse.parameter_types.clear();
        FrontendMessage::Parse(parse)
    }

    fn extended_bind_error(
        &self,
        mut bind: pg_proto::Bind,
        response: &pg_proto::DiagnosticResponse,
    ) -> FrontendMessage {
        bind.statement = bytes::Bytes::from(self.extended_error_marker(response));
        FrontendMessage::Bind(bind)
    }

    fn extended_error_marker(&self, response: &pg_proto::DiagnosticResponse) -> String {
        let code = response
            .fields
            .iter()
            .find(|field| field.code == b'C')
            .map(|field| String::from_utf8_lossy(&field.value))
            .unwrap_or_default();
        let message = response
            .fields
            .iter()
            .find(|field| field.code == b'M')
            .map(|field| String::from_utf8_lossy(&field.value))
            .unwrap_or_default();
        let message = message
            .chars()
            .filter(|character| character.is_ascii_graphic() || *character == ' ')
            .take(40)
            .collect::<String>();
        format!("__cipherstash_proxy_error_{code}_{message}")
    }
}

#[cfg(test)]
mod tests {
    use super::{quote_literal, Frontend};
    use crate::config::TandemConfig;
    use crate::error::{EncryptError, Error, MappingError};
    use crate::postgresql::context::statement::{OutputParam, OutputParamSource};
    use crate::postgresql::context::Statement;
    use crate::postgresql::context::{Context, KeysetIdentifier};
    use crate::postgresql::error_handler::PostgreSqlErrorHandler;
    use crate::postgresql::inbound_eql::InboundEql;
    use crate::postgresql::test_operation_id as operation_id;
    use crate::postgresql::Column;
    use crate::proxy::{EncryptConfig, EncryptionService};
    use crate::Identifier;
    use cipherstash_client::eql::{EncryptedPayloadV3, EQL_SCHEMA_VERSION_V3};
    use cipherstash_client::schema::{ColumnConfig, ColumnMode, ColumnType};
    use cipherstash_client::zerokms::EncryptedRecord;
    use eql_mapper::EqlTermVariant;
    use eql_mapper::Schema;
    use pg_proto::{Bind, Describe, DescribeTarget, FrontendMessage, Parse};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct TestService {
        fail_encrypt: bool,
    }

    #[async_trait::async_trait]
    impl EncryptionService for TestService {
        async fn encrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
            _columns: &[Option<Column>],
        ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
            if self.fail_encrypt {
                return Err(EncryptError::InvalidInboundEqlPayload.into());
            }
            Ok(Vec::new())
        }

        async fn decrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(Vec::new())
        }

        async fn decrypt_inbound_eql(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(Vec::new())
        }
    }

    fn frontend() -> Frontend<TestService> {
        frontend_with_config(TandemConfig::for_testing()).0
    }

    fn frontend_with_config(config: TandemConfig) -> (Frontend<TestService>, Context<TestService>) {
        frontend_with_service(
            config,
            TestService {
                fail_encrypt: false,
            },
        )
    }

    fn frontend_with_service(
        config: TandemConfig,
        service: TestService,
    ) -> (Frontend<TestService>, Context<TestService>) {
        frontend_with_encrypt_config_and_service(config, EncryptConfig::default(), service)
    }

    fn frontend_with_encrypt_config_and_service(
        config: TandemConfig,
        encrypt_config: EncryptConfig,
        service: TestService,
    ) -> (Frontend<TestService>, Context<TestService>) {
        let (reload_sender, _) = mpsc::unbounded_channel();
        let context = Context::new(
            1,
            Arc::new(config),
            Arc::new(encrypt_config),
            Arc::new(Schema::new("public")),
            Arc::new(rustls::RootCertStore::empty()),
            service,
            reload_sender,
        );
        (Frontend::new(context.clone()), context)
    }

    fn inbound_storage_payload() -> crate::EqlCiphertext {
        crate::EqlCiphertext::Encrypted(EncryptedPayloadV3 {
            version: EQL_SCHEMA_VERSION_V3,
            identifier: crate::Identifier::new("users", "email"),
            ciphertext: EncryptedRecord {
                iv: Default::default(),
                ciphertext: vec![1; 16],
                tag: vec![2; 16],
                descriptor: "users/email".into(),
                keyset_id: Some(uuid::Uuid::nil()),
                decryption_policy: None,
            },
            hmac_256: None,
            bloom_filter: None,
            ore_block_u64_8_256: None,
            ope_cllw: None,
        })
    }

    #[test]
    fn exception_literals_escape_quotes_and_backslashes() {
        assert_eq!(quote_literal("can't"), "'can''t'");
        assert_eq!(quote_literal(r"path\file"), r"E'path\\file'");
    }

    #[test]
    fn simple_errors_are_forwarded_as_exception_queries() {
        let frontend = frontend();
        let response = frontend.error_to_response(MappingError::CouldNotParseParameter.into());
        let output = frontend.simple_error_query(&response);

        assert!(matches!(
            output,
            FrontendMessage::Query(query)
                if String::from_utf8_lossy(&query).contains("RAISE EXCEPTION")
        ));
    }

    #[tokio::test]
    async fn failed_simple_query_releases_its_metrics_scope() {
        let (mut frontend, context) = frontend_with_config(TandemConfig::for_testing());

        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(bytes::Bytes::from_static(b"select 'unterminated")),
            )
            .await
            .unwrap();

        assert_eq!(context.active_metrics_scopes().unwrap(), 0);
    }

    #[test]
    fn extended_errors_preserve_the_original_protocol_message_kind() {
        let frontend = frontend();
        let parse = Parse {
            statement: bytes::Bytes::from_static(b"statement"),
            query: bytes::Bytes::from_static(b"select 1"),
            parameter_types: Vec::new(),
        };
        let bind = Bind {
            portal: bytes::Bytes::from_static(b"portal"),
            statement: bytes::Bytes::from_static(b"statement"),
            parameter_formats: Vec::new(),
            parameters: Vec::new(),
            result_formats: Vec::new(),
        };
        let response = frontend.error_to_response(MappingError::CouldNotParseParameter.into());

        assert!(matches!(
            frontend.extended_parse_error(parse, &response),
            FrontendMessage::Parse(_)
        ));
        assert!(matches!(
            frontend.extended_bind_error(bind, &response),
            FrontendMessage::Bind(_)
        ));
    }

    #[tokio::test]
    async fn stored_inbound_eql_without_a_destination_column_fails_closed() {
        let mut encrypted = vec![None];
        let result = frontend()
            .merge_inbound_eql(
                &mut encrypted,
                vec![Some(InboundEql::Store(inbound_storage_payload()))],
                &[None],
            )
            .await;

        assert!(matches!(
            result,
            Err(Error::Encrypt(EncryptError::InvalidInboundEqlPayload))
        ));
    }

    #[tokio::test]
    async fn mapping_disabled_extended_protocol_does_not_create_statement_metrics() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let (mut frontend, context) = frontend_with_config(config);
        let statement = bytes::Bytes::from_static(b"statement");
        let portal = bytes::Bytes::from_static(b"portal");

        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Parse(Parse {
                    statement: statement.clone(),
                    query: bytes::Bytes::from_static(b"select 1"),
                    parameter_types: Vec::new(),
                }),
            )
            .await
            .unwrap();
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Bind(Bind {
                    portal: portal.clone(),
                    statement: statement.clone(),
                    parameter_formats: Vec::new(),
                    parameters: Vec::new(),
                    result_formats: Vec::new(),
                }),
            )
            .await
            .unwrap();

        assert!(context
            .get_statement_metrics_scope(&statement)
            .unwrap()
            .is_none());
        assert!(context
            .get_portal_metrics_scope_id(&portal)
            .unwrap()
            .is_none());
        assert_eq!(context.active_metrics_scopes().unwrap(), 0);
    }

    #[tokio::test]
    async fn portals_bound_from_one_statement_have_isolated_parameter_metrics() {
        let (mut frontend, context) = frontend_with_config(TandemConfig::for_testing());
        let statement = bytes::Bytes::from_static(b"statement");
        let first_portal = bytes::Bytes::from_static(b"first_portal");
        let second_portal = bytes::Bytes::from_static(b"second_portal");
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Parse(Parse {
                    statement: statement.clone(),
                    query: bytes::Bytes::from_static(b"select $1::text"),
                    parameter_types: Vec::new(),
                }),
            )
            .await
            .unwrap();

        for (portal, parameter) in [
            (first_portal.clone(), bytes::Bytes::from_static(b"a")),
            (
                second_portal.clone(),
                bytes::Bytes::from_static(b"different"),
            ),
        ] {
            frontend
                .intercept(
                    operation_id(),
                    FrontendMessage::Bind(Bind {
                        portal,
                        statement: statement.clone(),
                        parameter_formats: Vec::new(),
                        parameters: vec![Some(parameter)],
                        result_formats: Vec::new(),
                    }),
                )
                .await
                .unwrap();
        }

        let first_scope = context
            .get_portal_metrics_scope_id(&first_portal)
            .unwrap()
            .unwrap();
        let second_scope = context
            .get_portal_metrics_scope_id(&second_portal)
            .unwrap()
            .unwrap();
        assert_ne!(first_scope, second_scope);
        assert_eq!(
            context
                .get_metrics_scope(first_scope)
                .unwrap()
                .unwrap()
                .metadata
                .param_bytes,
            1
        );
        assert_eq!(
            context
                .get_metrics_scope(second_scope)
                .unwrap()
                .unwrap()
                .metadata
                .param_bytes,
            9
        );
    }

    #[tokio::test]
    async fn failed_bind_releases_its_portal_metrics_scope() {
        let column_config = ColumnConfig {
            name: "secret".to_owned(),
            in_place: false,
            cast_type: ColumnType::Json,
            indexes: vec![],
            mode: ColumnMode::PlaintextDuplicate,
        };
        let column = Column {
            identifier: Identifier::new("records", "secret"),
            config: column_config.clone(),
            postgres_type: postgres_types::Type::JSONB,
            eql_term: EqlTermVariant::JsonValueSelector,
        };
        let mut encrypt_config = EncryptConfig::default();
        encrypt_config.insert(Identifier::new("records", "secret"), column_config);
        let (mut frontend, mut context) = frontend_with_encrypt_config_and_service(
            TandemConfig::for_testing(),
            encrypt_config,
            TestService { fail_encrypt: true },
        );
        let statement_name = bytes::Bytes::from_static(b"statement");
        let template_scope = context.start_metrics_scope().unwrap();
        context
            .set_statement_metrics_scope(statement_name.clone(), template_scope)
            .unwrap();
        context
            .add_statement(
                statement_name.clone(),
                Statement::new(
                    vec![Some(column.clone())],
                    vec![OutputParam {
                        column: Some(column),
                        source: OutputParamSource::Input(0),
                        query_operand: false,
                    }],
                    vec![],
                    vec![],
                    vec![],
                ),
            )
            .unwrap();

        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Bind(Bind {
                    portal: bytes::Bytes::from_static(b"portal"),
                    statement: statement_name,
                    parameter_formats: Vec::new(),
                    parameters: vec![Some(bytes::Bytes::from_static(b"\"value\""))],
                    result_formats: Vec::new(),
                }),
            )
            .await
            .unwrap();

        assert_eq!(context.active_metrics_scopes().unwrap(), 1);
    }

    #[tokio::test]
    async fn parse_registers_non_execution_error_correlation() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let (mut frontend, context) = frontend_with_config(config);
        let operation = operation_id();

        frontend
            .intercept(
                operation,
                FrontendMessage::Parse(Parse {
                    statement: bytes::Bytes::from_static(b"statement"),
                    query: bytes::Bytes::from_static(b"select 1"),
                    parameter_types: Vec::new(),
                }),
            )
            .await
            .unwrap();

        assert!(context
            .finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Failed,
            )
            .is_ok());
    }

    #[tokio::test]
    async fn bind_registers_non_execution_error_correlation() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let (mut frontend, context) = frontend_with_config(config);
        let operation = operation_id();

        frontend
            .intercept(
                operation,
                FrontendMessage::Bind(Bind {
                    portal: bytes::Bytes::from_static(b"portal"),
                    statement: bytes::Bytes::from_static(b"statement"),
                    parameter_formats: Vec::new(),
                    parameters: Vec::new(),
                    result_formats: Vec::new(),
                }),
            )
            .await
            .unwrap();

        assert!(context
            .finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Failed,
            )
            .is_ok());
    }

    #[tokio::test]
    async fn connection_disabled_mapping_allows_bound_portals_to_be_described() {
        let (mut frontend, _) = frontend_with_config(TandemConfig::for_testing());
        let portal = bytes::Bytes::from_static(b"portal");

        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(bytes::Bytes::from_static(
                    b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                )),
            )
            .await
            .unwrap();
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Bind(Bind {
                    portal: portal.clone(),
                    statement: bytes::Bytes::from_static(b"unknown_statement"),
                    parameter_formats: Vec::new(),
                    parameters: Vec::new(),
                    result_formats: Vec::new(),
                }),
            )
            .await
            .unwrap();

        let result = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Describe(Describe {
                    target: DescribeTarget::Portal,
                    name: portal,
                }),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn mapping_disabled_simple_query_waits_for_every_statement_completion() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let (mut frontend, context) = frontend_with_config(config);
        let operation = operation_id();

        frontend
            .intercept(
                operation,
                FrontendMessage::Query(bytes::Bytes::from_static(b"SELECT 1; SELECT 2")),
            )
            .await
            .unwrap();

        context
            .finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Completed,
            )
            .unwrap();
        assert!(context.get_execute(operation).unwrap().is_some());

        context
            .finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Completed,
            )
            .unwrap();
        assert!(context.get_execute(operation).unwrap().is_some());
        context
            .ready_for_query(pg_proto::TransactionStatus::Idle, Some(operation))
            .unwrap();
        assert!(matches!(
            context.get_execute(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[tokio::test]
    async fn mapping_disabled_unparsed_simple_query_waits_for_readiness() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let (mut frontend, context) = frontend_with_config(config);
        let operation = operation_id();

        frontend
            .intercept(
                operation,
                FrontendMessage::Query(bytes::Bytes::from_static(
                    b"DO $$ BEGIN RAISE NOTICE 'x'; END $$; SELECT 1",
                )),
            )
            .await
            .unwrap();

        context
            .finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Completed,
            )
            .unwrap();
        assert!(context.get_execute(operation).unwrap().is_some());

        context
            .ready_for_query(pg_proto::TransactionStatus::Idle, Some(operation))
            .unwrap();
        assert!(matches!(
            context.get_execute(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[tokio::test]
    async fn unknown_describe_target_is_forwarded_to_postgresql() {
        let mut frontend = frontend();

        let result = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Describe(Describe {
                    target: DescribeTarget::Portal,
                    name: bytes::Bytes::from_static(b"unknown_portal"),
                }),
            )
            .await;

        assert!(result.is_ok());
    }
}
