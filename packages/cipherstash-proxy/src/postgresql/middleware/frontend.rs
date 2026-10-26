use super::super::context::phase_timing::PhaseTimer;
use super::super::context::{Context, Deallocation, SessionId, Statement};
use super::super::error_handler::PostgreSqlErrorHandler;
use super::super::parser::SqlParser;
use super::super::rewrite::bind::Bind;
use super::super::rewrite::Name;
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
use eql_mapper::{
    self, EqlMapperError, EqlTermVariant, JsonSelectorSegment, SchemaError, TableResolver,
    TypeCheckedStatement, UnresolvedTable,
};
use metrics::{counter, histogram};
use pg_proto::{Describe, Execute, FrontendMessage, FrontendMiddlewareOutput, Parse};
use serde::Serialize;
use sqltk::parser::ast::{self, Value};
use sqltk::NodeKey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, warn};

/// What Proxy does with one statement, decided by [`Frontend::route`]. A
/// refusal is the `Err` of that decision rather than a variant, so it cannot be
/// matched past.
// One short-lived Route per statement in a message, so its size doesn't matter.
#[allow(clippy::large_enum_variant)]
enum Route<'a> {
    /// Encrypt and rewrite the statement.
    Map {
        typed: TypeCheckedStatement<'a>,
        statement: Statement,
    },
    /// Forward the statement as the client wrote it.
    Forward(Forward),
}

impl Route<'_> {
    fn is_map(&self) -> bool {
        matches!(self, Route::Map { .. })
    }

    fn is_fallback(&self) -> bool {
        matches!(self, Route::Forward(Forward::Fallback(_)))
    }
}

/// Why a statement is forwarded as written. Only `NothingEncrypted` is backed
/// by a type check; the others are policy, and each is named so it can be
/// found and tested.
enum Forward {
    /// `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` on this connection.
    MappingDisabled,
    /// A statement kind EQL Mapper does not type check.
    NotTypeChecked,
    /// Type checked, and it touches no encrypted column.
    NothingEncrypted,
    /// The type-check fallback, carrying the error it overrode.
    Fallback(Error),
}

impl Forward {
    fn record(&self) {
        match self {
            Forward::MappingDisabled => {
                warn!(msg = "Encrypted statement mapping is not enabled");
                counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            }
            Forward::NotTypeChecked => {
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            }
            Forward::NothingEncrypted => {
                debug!(target: MAPPER, msg = "Passthrough Statement");
                counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            }
            // `type_check` already logged and counted the error.
            Forward::Fallback(_) => {}
        }
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
        // PostgreSQL processes Terminate even while ignoring a failed batch.
        // Forward it so the driver can close both sides without waiting for Sync.
        if matches!(protocol_message, FrontendMessage::Terminate) {
            return Ok(FrontendMiddlewareOutput::Forward(protocol_message));
        }
        if self.failed_extended_batch || self.context.discarding_extended_messages()? {
            if matches!(protocol_message, FrontendMessage::Sync) {
                self.failed_extended_batch = false;
                self.context.sync(operation)?;
                return Ok(FrontendMiddlewareOutput::Forward(protocol_message));
            }
            self.context.discard_operation(operation)?;
            return Ok(FrontendMiddlewareOutput::Suppress(protocol_message));
        }

        if self.context.mapping_disabled() {
            match &protocol_message {
                FrontendMessage::Query(_) => self.context.unmapped_query(operation)?,
                FrontendMessage::Bind(bind) => {
                    self.context
                        .unmapped_bind(operation, bind.portal.clone(), &bind.statement)?
                }
                FrontendMessage::Execute(execute) => {
                    self.context.execute(operation, &execute.portal)?;
                }
                FrontendMessage::Describe(describe) => {
                    self.context.set_describe(operation, describe.clone())?
                }
                FrontendMessage::Close(close) => self.context.close(operation, close)?,
                FrontendMessage::Parse(parse) => {
                    self.context.unmapped_parse(operation, &parse.statement)?
                }
                FrontendMessage::FunctionCall(_) => self.context.function_call(operation)?,
                FrontendMessage::Sync => self.context.unmapped_sync(operation)?,
                _ => {}
            }
            return Ok(FrontendMiddlewareOutput::Forward(protocol_message));
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
                        warn!(
                            client_id = self.context.client_id,
                            msg = "Query Handler Error",
                            error = ?err.to_string(),
                        );
                        let response = self.error_to_response(err);
                        self.context
                            .refuse_query(operation, session_id, response.clone())?;
                        outbound_message = self.simple_error_query(&response);
                    }
                }
            }
            FrontendMessage::Describe(describe) => {
                self.describe_handler(operation, describe).await?;
            }
            FrontendMessage::Execute(execute) => {
                match self.context.execute(operation, &execute.portal) {
                    Ok(flush) => flush_after_forward = flush,
                    Err(err) => {
                        let response = self.error_to_response(err);
                        self.context.refuse_execute(operation, response.clone())?;
                        self.failed_extended_batch = true;
                        outbound_message = FrontendMessage::Execute(Execute {
                            // The client must not be able to pre-create the
                            // refusal portal and turn an error into execution.
                            portal: Self::refusal_name(),
                            max_rows: execute.max_rows,
                        });
                        flush_after_forward = true;
                    }
                }
            }
            FrontendMessage::Parse(parse) => {
                let failed_parse = parse.clone();
                match self.parse_handler(operation, parse).await {
                    Ok(Some(mapped)) => {
                        outbound_message = mapped;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        warn!(
                            client_id = self.context.client_id,
                            msg = "Parse Handler Error",
                            error = ?err.to_string(),
                        );
                        let response = self.error_to_response(err);
                        self.context.refuse_parse(operation, response.clone())?;
                        self.failed_extended_batch = true;
                        outbound_message = self.extended_parse_error(failed_parse, &response);
                    }
                }
            }
            FrontendMessage::Bind(bind) => {
                let failed_bind = bind.clone();
                let (portal, statement) = (bind.portal.clone(), bind.statement.clone());
                match self.bind_handler(operation, Bind::try_from(bind)?).await {
                    Ok(Some(mapped)) => {
                        self.context.bind_statement_effects(&portal, &statement)?;
                        outbound_message = mapped;
                    }
                    // No mapping needed, don't change the bytes
                    Ok(None) => {
                        self.context.bind_statement_effects(&portal, &statement)?;
                    }
                    Err(err) => match err {
                        Error::Mapping(MappingError::InvalidParameter(_)) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "EncryptError::InvalidParameter",
                            );
                            let response = self.error_to_response(err);
                            self.context.refuse_bind(operation, response.clone())?;
                            self.failed_extended_batch = true;
                            outbound_message = self.extended_bind_error(failed_bind, &response);
                        }
                        Error::Encrypt(EncryptError::UnknownKeysetIdentifier { .. }) => {
                            warn!(target: PROTOCOL,
                                client_id = self.context.client_id,
                                msg = "EncryptError::UnknownKeysetIdentifier",
                            );
                            let response = self.error_to_response(err);
                            self.context.refuse_bind(operation, response.clone())?;
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
                            self.context.refuse_bind(operation, response.clone())?;
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
                self.context.sync(operation)?;
            }
            FrontendMessage::Close(close) => {
                self.context.close(operation, &close)?;
            }
            FrontendMessage::FunctionCall(_) => {
                self.context.function_call(operation)?;
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
        for statement in &parsed_statements {
            self.context.validate_setting(statement)?;
        }
        if self
            .context
            .settings_simple_query_requires_fail_closed(&parsed_statements)?
        {
            return Err(MappingError::DependentStatementAfterSettings.into());
        }
        self.context.set_deallocations(
            operation,
            parsed_statements
                .iter()
                .filter_map(to_deallocation)
                .collect(),
        )?;
        self.context.prepare_schema_for_statement().await?;
        if self
            .context
            .simple_query_requires_fail_closed(&parsed_statements)
        {
            return Err(MappingError::DependentStatementAfterDdl.into());
        }
        if parsed_statements.iter().any(eql_mapper::requires_schema) {
            self.context.wait_for_schema_execution().await;
            self.context.ensure_schema_modelled()?;
        }
        let mut forwarded_statements = vec![];

        debug!(target: MAPPER,
            client_id = self.context.client_id,
            statements = parsed_statements.len(),
        );

        // Route every statement before acting on any, so the batch rule below
        // sees them all and no statement goes unchecked behind a fallback.
        let mut routes = parsed_statements
            .iter()
            .zip(self.context.batch_table_resolvers(&parsed_statements))
            .map(|(statement, resolver)| self.route(statement, vec![], &resolver))
            .collect::<Result<Vec<_>, _>>()?;

        // Proxy writes out every statement again when it rewrites a Query, and
        // a `COPY` written out again is not the one the client sent.
        if routes.iter().any(Route::is_map)
            && parsed_statements
                .iter()
                .any(|statement| matches!(statement, ast::Statement::Copy { .. }))
        {
            return Err(MappingError::CopyWithEncryptedStatement.into());
        }

        // A fallback forwards the whole Query as the client wrote it, so it
        // must not carry a statement that needs mapping: that statement's
        // values would reach the database unencrypted.
        if routes.iter().any(Route::is_map) {
            if let Some(index) = routes.iter().position(Route::is_fallback) {
                let Route::Forward(Forward::Fallback(err)) = routes.swap_remove(index) else {
                    unreachable!("position found a fallback");
                };
                return Err(err);
            }
        }

        let mut portal = Portal::passthrough(Some(session_id));
        let mut encrypted = false;
        let mut parse_duration_recorded = false;

        for (statement, route) in parsed_statements.iter().zip(routes) {
            match route {
                Route::Map {
                    typed: typed_statement,
                    statement: encryptable,
                } => {
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
                                &encryptable.literal_columns,
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
                    portal = Portal::encrypted(Arc::new(encryptable), Some(session_id));
                    self.context.update_statement_metadata(session_id, |m| {
                        m.encrypted = true;
                    })?;
                }
                Route::Forward(reason) => {
                    reason.record();
                    forwarded_statements.push(statement.clone());
                }
            }
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

        self.context
            .register_query(operation, session_id, portal, &forwarded_statements)?;

        if encrypted {
            let transformed_statement = forwarded_statements
                .iter()
                .map(SqlParser::statement_sql)
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
        operation: OperationId,
        mut message: Parse,
    ) -> Result<Option<FrontendMessage>, Error> {
        let original_query = message.query.clone();
        let session_id = self
            .context
            .begin_parse(operation, message.statement.clone())?;
        let parse_timer = PhaseTimer::start();

        let mut statement_text = String::from_utf8_lossy(&message.query).into_owned();

        let statement = SqlParser::parse_statement(&statement_text)?;
        if let Some(deallocation) = to_deallocation(&statement) {
            self.context
                .set_statement_deallocation(message.statement.to_owned(), deallocation)?;
        }

        self.context
            .prepare_parse(operation, &statement, &statement_text)
            .await?;

        // Capture the parse message param_types
        // These override the underlying column type
        let client_param_types = message.parameter_types.clone();
        let param_types = client_param_types.iter().map(|oid| *oid as i32).collect();

        let mut parse_duration_recorded = false;

        match self.route(&statement, param_types, &self.context.get_table_resolver())? {
            Route::Map {
                typed: typed_statement,
                mut statement,
            } => {
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
                self.context.finish_parse(operation, Some(statement))?;
            }
            Route::Forward(reason) => {
                reason.record();
                self.context.finish_parse(operation, None)?;
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
        let knowledge =
            self.context
                .begin_bind(operation, bind.portal.clone(), &bind.prepared_statement)?;
        if knowledge.mapping_disabled {
            warn!(msg = "Encrypted statement mapping is not enabled");
            counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
            counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
            self.context
                .finish_bind(operation, Portal::passthrough(None))?;
            return Ok(None);
        }

        // Proxy stores a Statement only for statements that touch encrypted columns, so a name it
        // has no Parse for could be one whose values must be encrypted. Refuse it rather than
        // send the values as they are.
        if !self.context.is_prepared(&bind.prepared_statement)? {
            return Err(MappingError::UnknownPreparedStatement {
                name: String::from_utf8_lossy(&bind.prepared_statement).into_owned(),
            }
            .into());
        }

        let session_id = knowledge.metrics;

        let result = async {
            // Track param bytes for diagnostics
            let param_bytes: usize = bind.param_values.iter().map(|p| p.bytes.len()).sum();
            self.context
                .with_metrics_scope(session_id, |m| m.metadata.set_param_bytes(param_bytes))?;

            debug!(target: PROTOCOL, client_id = self.context.client_id, bind = ?bind);

            let mut portal = Portal::passthrough(session_id);

            if let Some(statement) = knowledge.statement {
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
            self.context.finish_bind(operation, portal)?;

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

    /// Decides what Proxy does with one statement: map it, forward it for a
    /// named reason, or refuse it (`Err`). This is the only place that decides,
    /// and refusing is the default: a failed type check is forwarded only when
    /// [`Self::may_fall_back`] allows it. `resolver` is the schema to type check
    /// the statement against.
    fn route<'a>(
        &self,
        statement: &'a ast::Statement,
        param_types: Vec<i32>,
        resolver: &Arc<TableResolver>,
    ) -> Result<Route<'a>, Error> {
        if self.context.unsafe_disable_mapping()? {
            return Ok(Route::Forward(Forward::MappingDisabled));
        }
        if let Some((table_name, columns)) = eql_mapper::copies_into_table(statement) {
            if self.context.has_encrypted_columns() {
                check_copy_into(table_name, columns, resolver)?;
            }
            return Ok(Route::Forward(Forward::NothingEncrypted));
        }
        if !eql_mapper::requires_type_check(statement) {
            // A write nested in a statement the type checker skips, such as
            // `COPY (INSERT ... RETURNING ...) TO STDOUT`, is never mapped.
            if self.context.has_encrypted_columns() && eql_mapper::writes_data(statement) {
                return Err(MappingError::StatementCouldNotBeTypeChecked(
                    "it writes data inside a statement Proxy does not type check".to_owned(),
                )
                .into());
            }
            return Ok(Route::Forward(Forward::NotTypeChecked));
        }
        match self.type_check(statement, resolver) {
            Ok(typed) => match self.to_encryptable_statement(&typed, param_types)? {
                Some(encryptable) => Ok(Route::Map {
                    typed,
                    statement: encryptable,
                }),
                None => Ok(Route::Forward(Forward::NothingEncrypted)),
            },
            Err(err) => {
                // The one place the write policy is checked: it decides both
                // the error a refused write gets and whether it may fall back.
                let writes_encrypted =
                    self.context.has_encrypted_columns() && eql_mapper::writes_data(statement);
                let err = self.mapping_error(err, writes_encrypted);
                if self.may_fall_back(&err, writes_encrypted) {
                    Ok(Route::Forward(Forward::Fallback(err)))
                } else {
                    Err(err)
                }
            }
        }
    }

    /// The type-check fallback: a read that hit a coverage gap, only while
    /// mapping errors are off. A write never falls back while encrypted columns
    /// are configured, since it could put plaintext in an encrypted column.
    fn may_fall_back(&self, err: &Error, writes_encrypted: bool) -> bool {
        !self.context.mapping_errors_enabled() && !writes_encrypted && err.is_coverage_gap()
    }

    fn type_check<'a>(
        &self,
        statement: &'a ast::Statement,
        resolver: &Arc<TableResolver>,
    ) -> Result<TypeCheckedStatement<'a>, EqlMapperError> {
        let typed_statement = eql_mapper::type_check(resolver.clone(), statement)?;
        debug!(target: MAPPER,
            client_id = self.context.client_id,
            typed_statement = ?typed_statement
        );
        Ok(typed_statement)
    }

    /// The Proxy error for a statement that failed type checking.
    /// `writes_encrypted` is whether it writes data while encrypted columns
    /// are configured.
    fn mapping_error(&self, err: EqlMapperError, writes_encrypted: bool) -> Error {
        // A column this build cannot map is a refusal, not a coverage gap,
        // so it gets its own error, which `Error::is_coverage_gap` rejects.
        if let Some((table, column, column_type)) = err.as_unmappable_encrypted_column() {
            warn!(
                client_id = self.context.client_id,
                msg = "Statement refused: it references a column declared with a legacy EQL v2 type that this build cannot encrypt or decrypt. Migrate the column to an EQL v3 domain type.",
                table,
                column,
                column_type,
            );
            counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
            return MappingError::UnmappableEncryptedColumn {
                table: table.to_string(),
                column: column.to_string(),
                column_type: column_type.to_string(),
            }
            .into();
        }

        // The type checker understood the statement and knows the encrypted
        // column can't do it, so this is a refusal, not a coverage gap.
        if err.is_unsatisfied_bounds() {
            warn!(
                client_id = self.context.client_id,
                msg = "Statement refused: it uses an operation that an encrypted column's EQL type does not support.",
                error = err.to_string(),
            );
            counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
            return MappingError::UnsupportedEncryptedOperation(err.to_string()).into();
        }

        // The type checker never saw the table's columns, so a write to it
        // could put plaintext in an encrypted column.
        if let Some(unresolved) = err.as_unresolved_table().filter(|_| writes_encrypted) {
            warn!(
                client_id = self.context.client_id,
                msg = "Statement refused: it writes data and refers to a table Proxy cannot resolve, so Proxy cannot tell whether it writes to an encrypted column.",
                table = ?unresolved,
            );
            counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
            return match unresolved {
                UnresolvedTable::NotFound(table) => MappingError::UnresolvableTable {
                    table: table.to_string(),
                },
                UnresolvedTable::Ambiguous(table) => MappingError::AmbiguousTable {
                    table: table.to_string(),
                },
            }
            .into();
        }

        counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
        match err {
            EqlMapperError::InternalError(str) => {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Internal Error in EQL Mapper",
                    mapping_errors_enabled = self.context.mapping_errors_enabled(),
                    error = str,
                );
                MappingError::Internal(str).into()
            }
            err => {
                warn!(
                    client_id = self.context.client_id,
                    msg = "Unmappable statement",
                    mapping_errors_enabled = self.context.mapping_errors_enabled(),
                    error = err.to_string(),
                );
                MappingError::StatementCouldNotBeTypeChecked(err.to_string()).into()
            }
        }
    }
}

/// Refuses a `COPY ... FROM` into `columns` of `table_name`, or every column
/// when `columns` is empty, unless `resolver` shows none of them is encrypted.
/// Proxy does not encrypt copy data, and cannot tell whether a table or column
/// it has not loaded is encrypted.
fn check_copy_into(
    table_name: &ast::ObjectName,
    columns: &[ast::Ident],
    resolver: &TableResolver,
) -> Result<(), Error> {
    let table = resolver.resolve_table(table_name).map_err(|err| {
        let table = table_name.to_string();
        match err {
            SchemaError::AmbiguousTable(_) => MappingError::AmbiguousTable { table },
            _ => MappingError::UnresolvableTable { table },
        }
    })?;
    let unknown = |column: &ast::Ident| MappingError::CopyIntoUnknownColumn {
        table: table.name.value.clone(),
        column: column.value.clone(),
    };

    let targets = if columns.is_empty() {
        table.columns.clone()
    } else {
        columns
            .iter()
            .map(|column| table.get_column(column).map_err(|_| unknown(column)))
            .collect::<Result<_, _>>()?
    };
    // A defence: a table with no columns Proxy knows. A loaded table has its
    // columns, and DDL that leaves a table's columns unknown, such as
    // `CREATE TABLE ... AS`, fails closed before a `COPY` in the same Query.
    if targets.is_empty() {
        return Err(MappingError::UnresolvableTable {
            table: table_name.to_string(),
        }
        .into());
    }

    for column in targets {
        // The parser reads the `LIKE` clause of `CREATE TABLE t (LIKE u)` as a
        // column named `LIKE`, so Proxy does not know the columns it copies.
        // PostgreSQL reserves `LIKE`, so an unquoted column of that name is
        // always the clause.
        if column.name.quote_style.is_none() && column.name.value.eq_ignore_ascii_case("like") {
            return Err(MappingError::UnresolvableTable {
                table: table_name.to_string(),
            }
            .into());
        }
        if !matches!(column.kind, eql_mapper::ColumnKind::Native) {
            return Err(MappingError::CopyIntoEncryptedColumn {
                table: table.name.value.clone(),
                column: column.name.value.clone(),
            }
            .into());
        }
    }
    Ok(())
}

/// The prepared statements a `DEALLOCATE` or `DISCARD ALL` statement removes, if any
fn to_deallocation(statement: &ast::Statement) -> Option<Deallocation> {
    match statement {
        ast::Statement::Deallocate { name, .. } => {
            if name.quote_style.is_none() && name.value.eq_ignore_ascii_case("all") {
                Some(Deallocation::AllStatements)
            } else if name.quote_style.is_none() {
                // Unquoted identifiers are folded to lower case by PostgreSQL
                // (ASCII letters only, as `downcase_identifier` does for UTF-8)
                Some(Deallocation::Statement(Name::from(
                    name.value.to_ascii_lowercase(),
                )))
            } else {
                Some(Deallocation::Statement(Name::from(name.value.clone())))
            }
        }
        ast::Statement::Discard {
            object_type: ast::DiscardObject::ALL,
        } => Some(Deallocation::AllStatements),
        _ => None,
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
        _response: &pg_proto::DiagnosticResponse,
    ) -> FrontendMessage {
        // Backend replaces PostgreSQL's error with the refusal's diagnostic.
        // A predictable type name could refer to a type the client created,
        // turning the intended refusal into a successful Parse.
        let name = String::from_utf8_lossy(&Self::refusal_name()).into_owned();
        parse.query = bytes::Bytes::from(format!("SELECT NULL::\"{name}\""));
        parse.parameter_types.clear();
        FrontendMessage::Parse(parse)
    }

    fn extended_bind_error(
        &self,
        mut bind: pg_proto::Bind,
        _response: &pg_proto::DiagnosticResponse,
    ) -> FrontendMessage {
        // A predictable name could refer to a statement the client prepared,
        // turning the intended refusal into a successful Bind.
        bind.statement = Self::refusal_name();
        FrontendMessage::Bind(bind)
    }

    fn refusal_name() -> bytes::Bytes {
        // PostgreSQL named objects use NAMEDATALEN-byte hash keys. Put all
        // entropy inside the first 63 bytes, regardless of diagnostic length.
        bytes::Bytes::from(format!("__cipherstash_{}", uuid::Uuid::new_v4()))
    }
}

#[cfg(test)]
mod tests {
    use super::{quote_literal, to_deallocation, Frontend};
    use crate::config::TandemConfig;
    use crate::error::{EncryptError, Error, MappingError};
    use crate::postgresql::context::statement::{OutputParam, OutputParamSource};
    use crate::postgresql::context::Deallocation;
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
    fn refusal_names_differ_within_postgres_lookup_limit() {
        let frontend = frontend();
        let response = frontend.error_to_response(MappingError::PreparedSettingsChanged.into());
        let bind = Bind {
            portal: bytes::Bytes::new(),
            statement: bytes::Bytes::new(),
            parameter_formats: vec![],
            parameters: vec![],
            result_formats: vec![],
        };
        let FrontendMessage::Bind(first) = frontend.extended_bind_error(bind.clone(), &response)
        else {
            unreachable!()
        };
        let FrontendMessage::Bind(second) = frontend.extended_bind_error(bind, &response) else {
            unreachable!()
        };
        assert_ne!(
            &first.statement[..first.statement.len().min(63)],
            &second.statement[..second.statement.len().min(63)]
        );
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

    fn settings_frontend() -> (
        Frontend<TestService>,
        crate::postgresql::middleware::Backend<TestService>,
    ) {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        config.development.as_mut().unwrap().disable_mapping = false;
        config.development.as_mut().unwrap().enable_mapping_errors = true;
        let (frontend, context) = frontend_with_config(config);
        (
            frontend,
            crate::postgresql::middleware::Backend::new(context),
        )
    }

    async fn settings_query_succeeds(
        frontend: &mut Frontend<TestService>,
        backend: &mut crate::postgresql::middleware::Backend<TestService>,
        sql: &'static [u8],
        tag: &'static [u8],
        status: pg_proto::TransactionStatus,
    ) {
        let operation = operation_id();
        frontend
            .intercept(
                operation,
                FrontendMessage::Query(bytes::Bytes::from_static(sql)),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(operation),
                pg_proto::BackendMessage::CommandComplete(bytes::Bytes::from_static(tag)),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(operation),
                pg_proto::BackendMessage::ReadyForQuery(status),
            )
            .await
            .unwrap();
    }

    async fn assert_mapping_enabled(frontend: &mut Frontend<TestService>, enabled: bool) {
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(bytes::Bytes::from_static(
                    b"SELECT secret FROM table_that_does_not_exist",
                )),
            )
            .await
            .unwrap();
        let query = match output {
            pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query))
            | pg_proto::FrontendMiddlewareOutput::ForwardThenFlush(FrontendMessage::Query(query)) => {
                query
            }
            other => panic!("expected query output, got {other:?}"),
        };
        assert_eq!(
            String::from_utf8_lossy(&query).contains("RAISE EXCEPTION"),
            enabled
        );
    }

    #[tokio::test]
    async fn preparing_discard_does_not_reset_mapping_and_rejected_discard_preserves_it() {
        let (mut frontend, mut backend) = settings_frontend();
        settings_query_succeeds(
            &mut frontend,
            &mut backend,
            b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            b"SET",
            pg_proto::TransactionStatus::Idle,
        )
        .await;
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Parse(Parse {
                    statement: bytes::Bytes::from_static(b"discard"),
                    query: bytes::Bytes::from_static(b"DISCARD ALL"),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        assert_mapping_enabled(&mut frontend, false).await;
        settings_query_succeeds(
            &mut frontend,
            &mut backend,
            b"BEGIN",
            b"BEGIN",
            pg_proto::TransactionStatus::InTransaction,
        )
        .await;
        let discard = operation_id();
        frontend
            .intercept(
                discard,
                FrontendMessage::Query(bytes::Bytes::from_static(b"DISCARD ALL")),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(discard),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement(
                        "DISCARD ALL cannot run inside a transaction block".to_owned(),
                    ),
                ),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(discard),
                pg_proto::BackendMessage::ReadyForQuery(
                    pg_proto::TransactionStatus::FailedTransaction,
                ),
            )
            .await
            .unwrap();
        assert_mapping_enabled(&mut frontend, false).await;
    }

    #[tokio::test]
    async fn successful_discard_restores_mapping() {
        let (mut frontend, mut backend) = settings_frontend();
        settings_query_succeeds(
            &mut frontend,
            &mut backend,
            b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            b"SET",
            pg_proto::TransactionStatus::Idle,
        )
        .await;
        settings_query_succeeds(
            &mut frontend,
            &mut backend,
            b"DISCARD ALL",
            b"DISCARD ALL",
            pg_proto::TransactionStatus::Idle,
        )
        .await;
        assert_mapping_enabled(&mut frontend, true).await;
    }

    #[tokio::test]
    async fn ordinary_transaction_batch_is_forwarded_but_setting_before_mapping_is_refused() {
        let (mut frontend, _) = settings_frontend();
        let ordinary = bytes::Bytes::from_static(b"BEGIN; SELECT 1; COMMIT");
        let output = frontend
            .intercept(operation_id(), FrontendMessage::Query(ordinary.clone()))
            .await
            .unwrap();
        assert!(matches!(output,
            pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query))
            | pg_proto::FrontendMiddlewareOutput::ForwardThenFlush(FrontendMessage::Query(query)) if query == ordinary));
        let (mut frontend, _) = settings_frontend();
        let output = frontend.intercept(operation_id(), FrontendMessage::Query(bytes::Bytes::from_static(
            b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true; SELECT secret FROM table_that_does_not_exist"
        ))).await.unwrap();
        assert!(
            matches!(output, pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query))
            if String::from_utf8_lossy(&query).contains("RAISE EXCEPTION")
                && String::from_utf8_lossy(&query).contains(
                    "cannot change CipherStash settings before a statement that needs mapping"
                ))
        );
    }

    #[tokio::test]
    async fn skipped_setting_after_parse_error_does_not_change_mapping() {
        let (mut frontend, mut backend) = settings_frontend();
        let parse = operation_id();
        let name = bytes::Bytes::from_static(b"setting");
        frontend
            .intercept(
                parse,
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(
                        b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                    ),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Bind(Bind {
                    portal: name.clone(),
                    statement: name.clone(),
                    parameter_formats: vec![],
                    parameters: vec![],
                    result_formats: vec![],
                }),
            )
            .await
            .unwrap();
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: name,
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            output,
            pg_proto::FrontendMiddlewareOutput::ForwardThenFlush(_)
        ));
        let sync = operation_id();
        frontend
            .intercept(sync, FrontendMessage::Sync)
            .await
            .unwrap();
        backend
            .intercept(
                Some(parse),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement(
                        "prepared statement already exists".to_owned(),
                    ),
                ),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(sync),
                pg_proto::BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle),
            )
            .await
            .unwrap();
        assert_mapping_enabled(&mut frontend, true).await;
    }

    #[tokio::test]
    async fn statement_prepared_without_mapping_is_refused_after_mapping_is_enabled() {
        let (mut frontend, mut backend) = settings_frontend();
        settings_query_succeeds(
            &mut frontend,
            &mut backend,
            b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            b"SET",
            pg_proto::TransactionStatus::Idle,
        )
        .await;
        let name = bytes::Bytes::from_static(b"unmapped");
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(
                        b"SELECT secret FROM table_that_does_not_exist",
                    ),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        settings_query_succeeds(
            &mut frontend,
            &mut backend,
            b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false",
            b"SET",
            pg_proto::TransactionStatus::Idle,
        )
        .await;
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Bind(Bind {
                    portal: name.clone(),
                    statement: name.clone(),
                    parameter_formats: vec![],
                    parameters: vec![],
                    result_formats: vec![],
                }),
            )
            .await
            .unwrap();
        assert!(
            matches!(output, pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Bind(bind))
            if bind.statement != name && String::from_utf8_lossy(&bind.statement).contains("cipherstash"))
        );
    }

    #[tokio::test]
    async fn stale_portal_execute_returns_a_reprepare_error_without_executing_it() {
        let (mut frontend, mut backend) = settings_frontend();
        for (name, sql) in [
            (b"query".as_slice(), b"SELECT 1".as_slice()),
            (
                b"setting".as_slice(),
                b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true".as_slice(),
            ),
        ] {
            let parse = operation_id();
            frontend
                .intercept(
                    parse,
                    FrontendMessage::Parse(Parse {
                        statement: bytes::Bytes::copy_from_slice(name),
                        query: bytes::Bytes::copy_from_slice(sql),
                        parameter_types: vec![],
                    }),
                )
                .await
                .unwrap();
            backend
                .intercept(Some(parse), pg_proto::BackendMessage::ParseComplete)
                .await
                .unwrap();
            let bind = operation_id();
            frontend
                .intercept(
                    bind,
                    FrontendMessage::Bind(Bind {
                        portal: bytes::Bytes::copy_from_slice(name),
                        statement: bytes::Bytes::copy_from_slice(name),
                        parameter_formats: vec![],
                        parameters: vec![],
                        result_formats: vec![],
                    }),
                )
                .await
                .unwrap();
            backend
                .intercept(Some(bind), pg_proto::BackendMessage::BindComplete)
                .await
                .unwrap();
        }
        let setting = operation_id();
        frontend
            .intercept(
                setting,
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: bytes::Bytes::from_static(b"setting"),
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(setting),
                pg_proto::BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SET")),
            )
            .await
            .unwrap();
        let stale = operation_id();
        let output = frontend
            .intercept(
                stale,
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: bytes::Bytes::from_static(b"query"),
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        assert!(
            matches!(output, pg_proto::FrontendMiddlewareOutput::ForwardThenFlush(FrontendMessage::Execute(execute))
            if execute.portal != bytes::Bytes::from_static(b"query"))
        );
        let output = backend
            .intercept(
                Some(stale),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement(
                        "portal does not exist".to_owned(),
                    ),
                ),
            )
            .await
            .unwrap();
        assert!(
            matches!(output, pg_proto::BackendMiddlewareOutput::Forward(pg_proto::BackendMessage::ErrorResponse(response))
            if response.fields.iter().any(|field| field.code == b'M' && String::from_utf8_lossy(&field.value).contains("prepare and bind it again")))
        );
    }

    #[tokio::test]
    async fn rejected_duplicate_parse_does_not_forward_plaintext_to_original_encrypted_statement() {
        let mut schema = Schema::new("public");
        let mut table = eql_mapper::Table::new(sqltk::parser::ast::Ident::new("encrypted"));
        table.add_column(Arc::new(eql_mapper::Column::eql(
            sqltk::parser::ast::Ident::new("secret"),
            eql_mapper::EqlTraits::none(),
            eql_mapper::DomainIdentity::from_domain_name("eql_v3_text").unwrap(),
        )));
        schema.add_table(table);
        let mut encrypt_config = EncryptConfig::new();
        encrypt_config.insert(
            Identifier::new("encrypted", "secret"),
            ColumnConfig::build("secret".to_owned()).casts_as(ColumnType::Text),
        );
        let (reload_sender, _) = mpsc::unbounded_channel();
        let context = Context::new(
            1,
            Arc::new(TandemConfig::for_testing()),
            Arc::new(encrypt_config),
            Arc::new(schema),
            Arc::new(rustls::RootCertStore::empty()),
            TestService { fail_encrypt: true },
            reload_sender,
        );
        let mut frontend = Frontend::new(context.clone());
        let mut backend = crate::postgresql::middleware::Backend::new(context);
        let name = bytes::Bytes::from_static(b"encrypted_insert");
        let original = operation_id();
        let output = frontend
            .intercept(
                original,
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(b"INSERT INTO encrypted(secret) VALUES ($1)"),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            output,
            pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Parse(_))
        ));
        backend
            .intercept(Some(original), pg_proto::BackendMessage::ParseComplete)
            .await
            .unwrap();
        let duplicate = operation_id();
        frontend
            .intercept(
                duplicate,
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(b"SELECT 1"),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(duplicate),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement(
                        "prepared statement already exists".to_owned(),
                    ),
                ),
            )
            .await
            .unwrap();
        let sync = operation_id();
        frontend
            .intercept(sync, FrontendMessage::Sync)
            .await
            .unwrap();
        backend
            .intercept(
                Some(sync),
                pg_proto::BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle),
            )
            .await
            .unwrap();
        let bind = operation_id();
        let output = frontend
            .intercept(
                bind,
                FrontendMessage::Bind(Bind {
                    portal: name.clone(),
                    statement: name.clone(),
                    parameter_formats: vec![],
                    parameters: vec![Some(bytes::Bytes::from_static(b"private plaintext"))],
                    result_formats: vec![],
                }),
            )
            .await
            .unwrap();
        // With encryption metadata intact, the external cipher failure is
        // surfaced as a protocol error. Refusing stale prepared work is also
        // safe; forwarding this Bind to the original encrypted INSERT is not.
        assert!(
            matches!(output, pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Bind(bind))
            if bind.statement != name)
        );
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: name,
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            output,
            pg_proto::FrontendMiddlewareOutput::Suppress(FrontendMessage::Execute(_))
        ));
    }

    #[tokio::test]
    async fn terminate_is_forwarded_while_discarding_after_backend_or_local_error() {
        for local_error in [false, true] {
            let (mut frontend, mut backend) = settings_frontend();
            if local_error {
                frontend.failed_extended_batch = true;
            } else {
                let failed = operation_id();
                frontend
                    .context
                    .test_set_execute(failed, bytes::Bytes::new(), None)
                    .unwrap();
                backend
                    .intercept(
                        Some(failed),
                        pg_proto::BackendMessage::ErrorResponse(
                            crate::postgresql::diagnostics::invalid_sql_statement(
                                "execution failed".to_owned(),
                            ),
                        ),
                    )
                    .await
                    .unwrap();
            }
            let output = frontend
                .intercept(operation_id(), FrontendMessage::Terminate)
                .await
                .unwrap();
            assert!(
                matches!(
                    output,
                    pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Terminate)
                ),
                "local_error={local_error}: {output:?}"
            );
        }
    }

    #[tokio::test]
    async fn messages_after_backend_error_do_not_replace_statement() {
        let mut schema = Schema::new("public");
        let mut table = eql_mapper::Table::new(sqltk::parser::ast::Ident::new("encrypted"));
        table.add_column(Arc::new(eql_mapper::Column::eql(
            sqltk::parser::ast::Ident::new("secret"),
            eql_mapper::EqlTraits::none(),
            eql_mapper::DomainIdentity::from_domain_name("eql_v3_text").unwrap(),
        )));
        schema.add_table(table);
        let mut encrypt_config = EncryptConfig::new();
        encrypt_config.insert(
            Identifier::new("encrypted", "secret"),
            ColumnConfig::build("secret".to_owned()).casts_as(ColumnType::Text),
        );
        let (reload_sender, _) = mpsc::unbounded_channel();
        let context = Context::new(
            1,
            Arc::new(TandemConfig::for_testing()),
            Arc::new(encrypt_config),
            Arc::new(schema),
            Arc::new(rustls::RootCertStore::empty()),
            TestService { fail_encrypt: true },
            reload_sender,
        );
        let mut frontend = Frontend::new(context.clone());
        let mut backend = crate::postgresql::middleware::Backend::new(context);
        let name = bytes::Bytes::from_static(b"encrypted_insert");
        let original = operation_id();
        let output = frontend
            .intercept(
                original,
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(b"INSERT INTO encrypted(secret) VALUES ($1)"),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            output,
            pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Parse(_))
        ));
        backend
            .intercept(Some(original), pg_proto::BackendMessage::ParseComplete)
            .await
            .unwrap();
        let failed_execute = operation_id();
        frontend
            .context
            .test_set_execute(failed_execute, bytes::Bytes::new(), None)
            .unwrap();
        backend
            .intercept(
                Some(failed_execute),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement(
                        "execution failed".to_owned(),
                    ),
                ),
            )
            .await
            .unwrap();
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Close(pg_proto::Close {
                    name: name.clone(),
                    target: DescribeTarget::Statement,
                }),
            )
            .await
            .unwrap();
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(b"SELECT 1"),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        let sync = operation_id();
        frontend
            .intercept(sync, FrontendMessage::Sync)
            .await
            .unwrap();
        backend
            .intercept(
                Some(sync),
                pg_proto::BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle),
            )
            .await
            .unwrap();
        let bind = operation_id();
        let output = frontend
            .intercept(
                bind,
                FrontendMessage::Bind(Bind {
                    portal: name.clone(),
                    statement: name.clone(),
                    parameter_formats: vec![],
                    parameters: vec![Some(bytes::Bytes::from_static(b"private plaintext"))],
                    result_formats: vec![],
                }),
            )
            .await
            .unwrap();
        // With encryption metadata intact, the external cipher failure is
        // surfaced as a protocol error. Refusing stale prepared work is also
        // safe; forwarding this Bind to the original encrypted INSERT is not.
        assert!(
            matches!(output, pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Bind(bind))
            if bind.statement != name)
        );
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: name,
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            output,
            pg_proto::FrontendMiddlewareOutput::Suppress(FrontendMessage::Execute(_))
        ));
    }

    #[tokio::test]
    async fn rejected_named_parse_keeps_the_previous_setting_command() {
        let (mut frontend, mut backend) = settings_frontend();
        let name = bytes::Bytes::from_static(b"setting");
        let original = operation_id();
        frontend
            .intercept(
                original,
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(
                        b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                    ),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        backend
            .intercept(Some(original), pg_proto::BackendMessage::ParseComplete)
            .await
            .unwrap();
        let duplicate = operation_id();
        frontend
            .intercept(
                duplicate,
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(
                        b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false",
                    ),
                    parameter_types: vec![],
                }),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(duplicate),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement(
                        "prepared statement already exists".to_owned(),
                    ),
                ),
            )
            .await
            .unwrap();
        let sync = operation_id();
        frontend
            .intercept(sync, FrontendMessage::Sync)
            .await
            .unwrap();
        backend
            .intercept(
                Some(sync),
                pg_proto::BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle),
            )
            .await
            .unwrap();
        let bind = operation_id();
        frontend
            .intercept(
                bind,
                FrontendMessage::Bind(Bind {
                    portal: name.clone(),
                    statement: name.clone(),
                    parameter_formats: vec![],
                    parameters: vec![],
                    result_formats: vec![],
                }),
            )
            .await
            .unwrap();
        backend
            .intercept(Some(bind), pg_proto::BackendMessage::BindComplete)
            .await
            .unwrap();
        let execute = operation_id();
        frontend
            .intercept(
                execute,
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: name,
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(execute),
                pg_proto::BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SET")),
            )
            .await
            .unwrap();
        let sync = operation_id();
        let output = frontend
            .intercept(sync, FrontendMessage::Sync)
            .await
            .unwrap();
        assert!(matches!(
            output,
            pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Sync)
        ));
        backend
            .intercept(
                Some(sync),
                pg_proto::BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle),
            )
            .await
            .unwrap();
        assert_mapping_enabled(&mut frontend, false).await;
    }

    #[tokio::test]
    async fn executing_prepared_disable_mapping_changes_subsequent_queries() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        config.development.as_mut().unwrap().disable_mapping = false;
        config.development.as_mut().unwrap().enable_mapping_errors = true;
        let (mut frontend, context) = frontend_with_config(config);
        let mut backend = crate::postgresql::middleware::Backend::new(context);
        let name = bytes::Bytes::from_static(b"disable_mapping");
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Parse(Parse {
                    statement: name.clone(),
                    query: bytes::Bytes::from_static(
                        b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                    ),
                    parameter_types: Vec::new(),
                }),
            )
            .await
            .unwrap();
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Bind(Bind {
                    portal: name.clone(),
                    statement: name.clone(),
                    parameter_formats: Vec::new(),
                    parameters: Vec::new(),
                    result_formats: Vec::new(),
                }),
            )
            .await
            .unwrap();
        let execute = operation_id();
        frontend
            .intercept(
                execute,
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: name,
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(execute),
                pg_proto::BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SET")),
            )
            .await
            .unwrap();
        // A disabled connection forwards even SQL that the mapper cannot resolve.
        let sql = bytes::Bytes::from_static(b"SELECT secret FROM table_that_does_not_exist");
        let output = frontend
            .intercept(operation_id(), FrontendMessage::Query(sql.clone()))
            .await
            .unwrap();
        assert!(
            matches!(output, pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query)) if query == sql)
        );
    }

    #[tokio::test]
    async fn preparing_disable_mapping_does_not_bypass_statement_validation() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let development = config.development.as_mut().unwrap();
        development.disable_mapping = false;
        development.enable_mapping_errors = true;
        let (mut frontend, _) = frontend_with_config(config);
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Parse(Parse {
                    statement: bytes::Bytes::from_static(b"disable_mapping"),
                    query: bytes::Bytes::from_static(
                        b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                    ),
                    parameter_types: Vec::new(),
                }),
            )
            .await
            .unwrap();
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(bytes::Bytes::from_static(
                    b"SELECT secret FROM table_that_does_not_exist",
                )),
            )
            .await
            .unwrap();
        assert!(
            matches!(output, pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query))
            if String::from_utf8_lossy(&query).contains("RAISE EXCEPTION"))
        );
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
            .test_statement_metrics_scope(&statement)
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
            .test_set_statement_metrics_scope(statement_name.clone(), template_scope)
            .unwrap();
        context
            .test_add_statement(
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
            .test_finish_execution(
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
            .test_finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Failed,
            )
            .is_ok());
    }

    #[tokio::test]
    async fn connection_disabled_mapping_allows_bound_portals_to_be_described() {
        let (mut frontend, context) = frontend_with_config(TandemConfig::for_testing());
        let mut backend = crate::postgresql::middleware::Backend::new(context);
        let setting = operation_id();
        let portal = bytes::Bytes::from_static(b"portal");

        frontend
            .intercept(
                setting,
                FrontendMessage::Query(bytes::Bytes::from_static(
                    b"SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                )),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(setting),
                pg_proto::BackendMessage::CommandComplete(bytes::Bytes::from_static(b"SET")),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(setting),
                pg_proto::BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle),
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
            .test_finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Completed,
            )
            .unwrap();
        assert!(context.get_execute(operation).unwrap().is_some());

        context
            .test_finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Completed,
            )
            .unwrap();
        assert!(context.get_execute(operation).unwrap().is_some());
        context
            .test_ready_for_query(pg_proto::TransactionStatus::Idle, Some(operation))
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
            .test_finish_execution(
                operation,
                crate::postgresql::context::ExecutionOutcome::Completed,
            )
            .unwrap();
        assert!(context.get_execute(operation).unwrap().is_some());

        context
            .test_ready_for_query(pg_proto::TransactionStatus::Idle, Some(operation))
            .unwrap();
        assert!(matches!(
            context.get_execute(operation),
            Err(Error::Context(crate::error::ContextError::UnknownOperation))
        ));
    }

    #[tokio::test]
    async fn mapping_disabled_simple_query_error_does_not_suppress_next_query() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let (mut frontend, context) = frontend_with_config(config);
        let mut backend = crate::postgresql::middleware::Backend::new(context.clone());
        let failed = operation_id();

        frontend
            .intercept(
                failed,
                FrontendMessage::Query(bytes::Bytes::from_static(b"SELECT 1/0")),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(failed),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement(
                        "division by zero".to_owned(),
                    ),
                ),
            )
            .await
            .unwrap();
        // A simple Query's error must not start discarding until a Sync that will never come
        assert!(!context.discarding_extended_messages().unwrap());
        backend
            .intercept(
                Some(failed),
                pg_proto::BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle),
            )
            .await
            .unwrap();

        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(bytes::Bytes::from_static(b"SELECT 1")),
            )
            .await
            .unwrap();
        assert!(
            matches!(
                output,
                pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(_))
            ),
            "{output:?}"
        );
    }

    #[test]
    fn parse_refusal_names_differ_within_postgres_lookup_limit() {
        let frontend = frontend();
        let response = frontend.error_to_response(MappingError::PreparedSettingsChanged.into());
        let parse = Parse {
            statement: bytes::Bytes::new(),
            query: bytes::Bytes::from_static(b"SELECT 1"),
            parameter_types: vec![],
        };
        let refused_type = |message: FrontendMessage| {
            let FrontendMessage::Parse(parse) = message else {
                unreachable!()
            };
            let query = String::from_utf8(parse.query.to_vec()).unwrap();
            let name = query
                .split_once('"')
                .map(|(_, rest)| rest.trim_end_matches('"').replace("\"\"", "\""))
                .unwrap();
            name.as_bytes()[..name.len().min(63)].to_vec()
        };
        let first = refused_type(frontend.extended_parse_error(parse.clone(), &response));
        let second = refused_type(frontend.extended_parse_error(parse, &response));
        assert_ne!(first, second);
    }

    /// Drives one failed simple-protocol request through Frontend and Backend
    /// and returns how the next Query is handled.
    async fn next_query_after_failed_request(
        config: TandemConfig,
        request: Vec<FrontendMessage>,
        copy_in: bool,
    ) -> pg_proto::FrontendMiddlewareOutput {
        let (mut frontend, context) = frontend_with_config(config);
        let mut backend = crate::postgresql::middleware::Backend::new(context.clone());
        let failed = operation_id();
        let mut request = request.into_iter();
        frontend
            .intercept(failed, request.next().unwrap())
            .await
            .unwrap();
        if copy_in {
            backend
                .intercept(
                    Some(failed),
                    pg_proto::BackendMessage::CopyInResponse(pg_proto::CopyResponse {
                        overall_format: 0,
                        column_formats: vec![],
                    }),
                )
                .await
                .unwrap();
        }
        for message in request {
            frontend.intercept(operation_id(), message).await.unwrap();
        }
        let result = backend
            .intercept(
                Some(failed),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement("failed".to_owned()),
                ),
            )
            .await;
        assert!(result.is_ok(), "{result:?}");
        // The request is not extended protocol, so its error must not start discarding
        // until a Sync that will never come
        assert!(!context.discarding_extended_messages().unwrap());
        let result = backend
            .intercept(
                Some(failed),
                pg_proto::BackendMessage::ReadyForQuery(pg_proto::TransactionStatus::Idle),
            )
            .await;
        assert!(result.is_ok(), "{result:?}");
        frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(bytes::Bytes::from_static(b"SELECT 1")),
            )
            .await
            .unwrap()
    }

    fn function_call() -> FrontendMessage {
        FrontendMessage::FunctionCall(pg_proto::FunctionCall {
            function_oid: 952,
            argument_formats: vec![],
            arguments: vec![],
            result_format: 0,
        })
    }

    #[tokio::test]
    async fn failed_function_call_keeps_the_connection_usable() {
        let output = next_query_after_failed_request(
            TandemConfig::for_testing(),
            vec![function_call()],
            false,
        )
        .await;
        assert!(
            matches!(
                output,
                pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(_))
            ),
            "{output:?}"
        );
    }

    #[tokio::test]
    async fn failed_function_call_with_mapping_disabled_keeps_the_connection_usable() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let output = next_query_after_failed_request(config, vec![function_call()], false).await;
        assert!(
            matches!(
                output,
                pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(_))
            ),
            "{output:?}"
        );
    }

    #[tokio::test]
    async fn mapping_disabled_extended_error_after_sync_does_not_suppress_pipelined_query() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let (mut frontend, context) = frontend_with_config(config);
        let mut backend = crate::postgresql::middleware::Backend::new(context);
        let failed = operation_id();

        frontend
            .intercept(
                failed,
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: bytes::Bytes::new(),
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        frontend
            .intercept(operation_id(), FrontendMessage::Sync)
            .await
            .unwrap();
        let result = backend
            .intercept(
                Some(failed),
                pg_proto::BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement("failed".to_owned()),
                ),
            )
            .await;
        assert!(result.is_ok(), "{result:?}");

        // The client pipelined this Query after Sync, before the error arrived.
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(bytes::Bytes::from_static(b"SELECT 1")),
            )
            .await
            .unwrap();
        assert!(
            matches!(
                output,
                pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(_))
            ),
            "{output:?}"
        );
    }

    #[tokio::test]
    async fn simple_copy_fail_error_belongs_to_its_query() {
        let output = next_query_after_failed_request(
            TandemConfig::for_testing(),
            vec![
                FrontendMessage::Query(bytes::Bytes::from_static(b"COPY t FROM STDIN")),
                FrontendMessage::CopyFail(bytes::Bytes::from_static(b"cancelled")),
            ],
            true,
        )
        .await;
        assert!(
            matches!(
                output,
                pg_proto::FrontendMiddlewareOutput::Forward(FrontendMessage::Query(_))
            ),
            "{output:?}"
        );
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

    fn deallocation(sql: &str) -> Option<Deallocation> {
        let statement = crate::postgresql::parser::SqlParser::parse_statement(sql).unwrap();
        to_deallocation(&statement)
    }

    #[test]
    fn sql_that_removes_prepared_statements() {
        let statement = |name: &'static str| {
            Some(Deallocation::Statement(bytes::Bytes::from_static(
                name.as_bytes(),
            )))
        };
        assert_eq!(deallocation("DEALLOCATE _pg3_1"), statement("_pg3_1"));
        assert_eq!(deallocation("DEALLOCATE PREPARE a1"), statement("a1"));
        // Unquoted names are folded to lower case, quoted names are not
        assert_eq!(deallocation("DEALLOCATE S_1"), statement("s_1"));
        assert_eq!(deallocation(r#"DEALLOCATE "S_1""#), statement("S_1"));
        // Only ASCII letters are folded, as PostgreSQL does
        assert_eq!(deallocation("DEALLOCATE ÄB1"), statement("Äb1"));
        assert_eq!(
            deallocation("DEALLOCATE ALL"),
            Some(Deallocation::AllStatements)
        );
        assert_eq!(
            deallocation("DISCARD ALL"),
            Some(Deallocation::AllStatements)
        );
        assert_eq!(deallocation("DISCARD PLANS"), None);
        assert_eq!(deallocation("SELECT 1"), None);
    }
}

#[cfg(test)]
mod settings_reset_tests {
    //! PostgreSQL setting resets must also reset Proxy encryption controls.
    use super::Frontend;
    use crate::config::TandemConfig;
    use crate::error::{EncryptError, Error};
    use crate::postgresql::context::{Context, KeysetIdentifier};
    use crate::postgresql::middleware::Backend;
    use crate::postgresql::test_operation_id as operation_id;
    use crate::postgresql::Column;
    use crate::proxy::{EncryptConfig, EncryptionService};
    use bytes::Bytes;
    use cipherstash_client::{encryption::Plaintext, IdentifiedBy};
    use pg_proto::{BackendMessage, FrontendMessage, FrontendMiddlewareOutput, TransactionStatus};
    use std::sync::{Arc, Mutex};

    /// Records the keyset each encryption request asks for.
    #[derive(Clone, Default)]
    struct Recorder {
        selected: Arc<Mutex<Vec<Option<KeysetIdentifier>>>>,
    }

    #[async_trait::async_trait]
    impl EncryptionService for Recorder {
        async fn encrypt(
            &self,
            keyset: Option<KeysetIdentifier>,
            _: Vec<Option<Plaintext>>,
            _: &[Option<Column>],
        ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
            self.selected.lock().unwrap().push(keyset);
            Err(EncryptError::InvalidInboundEqlPayload.into())
        }
        async fn decrypt(
            &self,
            _: Option<KeysetIdentifier>,
            _: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<Plaintext>>, Error> {
            Ok(Vec::new())
        }
        async fn decrypt_inbound_eql(
            &self,
            _: Option<KeysetIdentifier>,
            _: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<Plaintext>>, Error> {
            Ok(Vec::new())
        }
    }

    fn client() -> (Frontend<Recorder>, Backend<Recorder>, Recorder) {
        let mut config = TandemConfig::for_testing();
        config.encrypt.default_keyset_id = None;
        let mut schema = eql_mapper::Schema::new("public");
        let mut table = eql_mapper::Table::new(sqltk::parser::ast::Ident::new("encrypted"));
        table.add_column(Arc::new(eql_mapper::Column::eql(
            sqltk::parser::ast::Ident::new("secret"),
            eql_mapper::EqlTraits::none(),
            eql_mapper::DomainIdentity::from_domain_name("eql_v3_text").unwrap(),
        )));
        schema.add_table(table);
        let mut encryption = EncryptConfig::new();
        encryption.insert(
            crate::Identifier::new("encrypted", "secret"),
            cipherstash_client::schema::ColumnConfig::build("secret".to_owned())
                .casts_as(cipherstash_client::schema::ColumnType::Text),
        );
        let recorder = Recorder::default();
        let (sender, _) = tokio::sync::mpsc::unbounded_channel();
        let context = Context::new(
            1,
            Arc::new(config),
            Arc::new(encryption),
            Arc::new(schema),
            Arc::new(rustls::RootCertStore::empty()),
            recorder.clone(),
            sender,
        );
        (
            Frontend::new(context.clone()),
            Backend::new(context),
            recorder,
        )
    }

    fn named(name: &str) -> Option<KeysetIdentifier> {
        Some(KeysetIdentifier(IdentifiedBy::Name(name.to_owned().into())))
    }

    async fn command(
        frontend: &mut Frontend<Recorder>,
        backend: &mut Backend<Recorder>,
        sql: &str,
        tag: &str,
        status: TransactionStatus,
    ) {
        let operation = operation_id();
        frontend
            .intercept(
                operation,
                FrontendMessage::Query(Bytes::copy_from_slice(sql.as_bytes())),
            )
            .await
            .unwrap();
        backend
            .intercept(
                Some(operation),
                BackendMessage::CommandComplete(Bytes::copy_from_slice(tag.as_bytes())),
            )
            .await
            .unwrap();
        backend
            .intercept(Some(operation), BackendMessage::ReadyForQuery(status))
            .await
            .unwrap();
    }

    /// PgCat cleans a server connection on which a client ran `SET` by sending
    /// `RESET ROLE;RESET ALL;` before giving it to another client. PostgreSQL
    /// resets its own `cipherstash.*` GUCs; Proxy must reset its settings too, or
    /// the next client inherits the previous tenant's keyset.
    #[tokio::test]
    async fn pgcat_reset_all_is_forwarded_and_clears_the_session_keyset() {
        let (mut frontend, mut backend, _) = client();
        command(
            &mut frontend,
            &mut backend,
            "SET CIPHERSTASH.KEYSET_NAME = 'tenant-a'",
            "SET",
            TransactionStatus::Idle,
        )
        .await;
        assert_eq!(
            frontend.context.keyset_identifier().unwrap(),
            named("tenant-a")
        );

        let reset = Bytes::from_static(b"RESET ROLE;RESET ALL;");
        let operation = operation_id();
        let output = frontend
            .intercept(operation, FrontendMessage::Query(reset.clone()))
            .await
            .unwrap();
        let forwarded = match output {
            FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query))
            | FrontendMiddlewareOutput::ForwardThenFlush(FrontendMessage::Query(query)) => query,
            other => panic!("expected a forwarded query, got {other:?}"),
        };
        assert_eq!(
            String::from_utf8_lossy(&forwarded),
            String::from_utf8_lossy(&reset)
        );
        for _ in 0..2 {
            backend
                .intercept(
                    Some(operation),
                    BackendMessage::CommandComplete(Bytes::from_static(b"RESET")),
                )
                .await
                .unwrap();
        }
        backend
            .intercept(
                Some(operation),
                BackendMessage::ReadyForQuery(TransactionStatus::Idle),
            )
            .await
            .unwrap();

        assert_eq!(frontend.context.keyset_identifier().unwrap(), None);
    }

    /// A keyset change in a simple-query batch must not be bypassed by a later
    /// statement that is encrypted with the keyset in effect before the batch.
    #[tokio::test]
    async fn keyset_setting_before_encrypted_statement_in_one_batch_is_refused() {
        let (mut frontend, _, recorder) = client();
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(Bytes::from_static(
                    b"SET CIPHERSTASH.KEYSET_NAME = 'tenant-b'; INSERT INTO encrypted(secret) VALUES ('value')",
                )),
            )
            .await
            .unwrap();
        let query = match output {
            FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query))
            | FrontendMiddlewareOutput::ForwardThenFlush(FrontendMessage::Query(query)) => query,
            other => panic!("expected a forwarded query, got {other:?}"),
        };
        assert_eq!(recorder.selected.lock().unwrap().len(), 0);
        assert!(String::from_utf8_lossy(&query)
            .contains("cannot change CipherStash settings before a statement that needs mapping"));
    }

    /// `RESET CIPHERSTASH.KEYSET_NAME` returns the keyset to its default.
    #[tokio::test]
    async fn reset_of_a_cipherstash_setting_clears_it() {
        let (mut frontend, mut backend, _) = client();
        command(
            &mut frontend,
            &mut backend,
            "SET CIPHERSTASH.KEYSET_NAME = 'tenant-a'",
            "SET",
            TransactionStatus::Idle,
        )
        .await;
        command(
            &mut frontend,
            &mut backend,
            "RESET CIPHERSTASH.KEYSET_NAME",
            "RESET",
            TransactionStatus::Idle,
        )
        .await;
        assert_eq!(frontend.context.keyset_identifier().unwrap(), None);
    }
}

/// A statement Proxy cannot type check is forwarded as written when
/// mapping errors are off. These pin when that fallback is refused instead.
#[cfg(test)]
mod unresolvable_table_tests {
    use super::Frontend;
    use crate::config::TandemConfig;
    use crate::error::{Error, MappingError};
    use crate::postgresql::context::{Context, KeysetIdentifier};
    use crate::postgresql::test_operation_id as operation_id;
    use crate::postgresql::Column;
    use crate::proxy::{EncryptConfig, EncryptionService};
    use bytes::Bytes;
    use cipherstash_client::encryption::Plaintext;
    use cipherstash_client::eql::{EncryptedPayloadV3, EQL_SCHEMA_VERSION_V3};
    use cipherstash_client::zerokms::EncryptedRecord;
    use pg_proto::{FrontendMessage, FrontendMiddlewareOutput, Parse};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    /// Encrypts every value as a stub ciphertext and counts the values.
    #[derive(Clone, Default)]
    struct Counter {
        encrypted: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl EncryptionService for Counter {
        async fn encrypt(
            &self,
            _: Option<KeysetIdentifier>,
            plaintexts: Vec<Option<Plaintext>>,
            _: &[Option<Column>],
        ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
            self.encrypted.fetch_add(plaintexts.len(), Ordering::SeqCst);
            Ok(plaintexts
                .iter()
                .map(|_| Some(crate::EqlOutput::Store(ciphertext())))
                .collect())
        }
        async fn decrypt(
            &self,
            _: Option<KeysetIdentifier>,
            _: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<Plaintext>>, Error> {
            Ok(Vec::new())
        }
        async fn decrypt_inbound_eql(
            &self,
            _: Option<KeysetIdentifier>,
            _: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<Plaintext>>, Error> {
            Ok(Vec::new())
        }
    }

    fn ciphertext() -> crate::EqlCiphertext {
        crate::EqlCiphertext::Encrypted(EncryptedPayloadV3 {
            version: EQL_SCHEMA_VERSION_V3,
            identifier: crate::Identifier::new("encrypted", "secret"),
            ciphertext: EncryptedRecord {
                iv: Default::default(),
                ciphertext: vec![1; 16],
                tag: vec![2; 16],
                descriptor: "encrypted/secret".into(),
                keyset_id: Some(uuid::Uuid::nil()),
                decryption_policy: None,
            },
            hmac_256: None,
            bloom_filter: None,
            ore_block_u64_8_256: None,
            ope_cllw: None,
        })
    }

    /// A connection whose schema has `encrypted (id, secret)` with `secret`
    /// encrypted, and an `encrypted` encrypt config only when `encrypted` is set,
    /// a native `notes (body)`, and when `encrypted` is set, `legacy (data)` with
    /// `data` a legacy EQL v2 column.
    fn client(config: TandemConfig, encrypted: bool) -> (Frontend<Counter>, Counter) {
        let mut schema = eql_mapper::Schema::new("public");
        let mut table = eql_mapper::Table::new(sqltk::parser::ast::Ident::new("encrypted"));
        table.add_column(Arc::new(eql_mapper::Column::native(
            sqltk::parser::ast::Ident::new("id"),
        )));
        let secret = sqltk::parser::ast::Ident::new("secret");
        table.add_column(Arc::new(if encrypted {
            eql_mapper::Column::eql(
                secret,
                eql_mapper::EqlTraits::none(),
                eql_mapper::DomainIdentity::from_domain_name("eql_v3_text").unwrap(),
            )
        } else {
            eql_mapper::Column::native(secret)
        }));
        schema.add_table(table);
        let mut notes = eql_mapper::Table::new(sqltk::parser::ast::Ident::new("notes"));
        notes.add_column(Arc::new(eql_mapper::Column::native(
            sqltk::parser::ast::Ident::new("body"),
        )));
        schema.add_table(notes);
        if encrypted {
            let mut legacy = eql_mapper::Table::new(sqltk::parser::ast::Ident::new("legacy"));
            legacy.add_column(Arc::new(eql_mapper::Column::unmappable_encrypted(
                sqltk::parser::ast::Ident::new("data"),
                "eql_v2_encrypted",
            )));
            schema.add_table(legacy);
        }
        let mut encryption = EncryptConfig::new();
        if encrypted {
            encryption.insert(
                crate::Identifier::new("encrypted", "secret"),
                cipherstash_client::schema::ColumnConfig::build("secret".to_owned())
                    .casts_as(cipherstash_client::schema::ColumnType::Text),
            );
        }
        let counter = Counter::default();
        let (sender, _) = tokio::sync::mpsc::unbounded_channel();
        let context = Context::new(
            1,
            Arc::new(config),
            Arc::new(encryption),
            Arc::new(schema),
            Arc::new(rustls::RootCertStore::empty()),
            counter.clone(),
            sender,
        );
        (Frontend::new(context), counter)
    }

    fn encrypted_client() -> (Frontend<Counter>, Counter) {
        client(TandemConfig::for_testing(), true)
    }

    /// Sends a simple Query and returns the SQL Proxy forwards for it.
    async fn query(frontend: &mut Frontend<Counter>, sql: &str) -> String {
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(Bytes::copy_from_slice(sql.as_bytes())),
            )
            .await
            .unwrap();
        match output {
            FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query))
            | FrontendMiddlewareOutput::ForwardThenFlush(FrontendMessage::Query(query)) => {
                String::from_utf8_lossy(&query).into_owned()
            }
            other => panic!("expected a forwarded query, got {other:?}"),
        }
    }

    fn unresolvable(table: &str) -> String {
        MappingError::UnresolvableTable {
            table: table.to_owned(),
        }
        .to_string()
    }

    fn assert_refused(forwarded: &str, table: &str) {
        // The message is quoted once in the DO block's body, then again as
        // the DO block's literal.
        assert!(
            forwarded.contains("RAISE EXCEPTION")
                && forwarded.contains(&unresolvable(table).replace('\'', "''''")),
            "expected a refusal naming {table}, forwarded: {forwarded}"
        );
        assert!(
            !forwarded.contains("plaintext"),
            "the plaintext reached PostgreSQL: {forwarded}"
        );
    }

    const WRITES: [(&str, &str); 6] = [
        (
            "INSERT INTO accounts (secret) VALUES ('plaintext')",
            "accounts",
        ),
        (
            "UPDATE accounts SET secret = 'plaintext' WHERE id = 1",
            "accounts",
        ),
        ("DELETE FROM accounts WHERE secret = 'plaintext'", "accounts"),
        (
            "INSERT INTO archive.encrypted (secret) VALUES ('plaintext')",
            "archive.encrypted",
        ),
        (
            "INSERT INTO encrypted (id, secret) SELECT id, 'plaintext' FROM accounts",
            "accounts",
        ),
        (
            "MERGE INTO accounts USING encrypted ON accounts.id = encrypted.id WHEN NOT MATCHED THEN INSERT (secret) VALUES ('plaintext')",
            "accounts",
        ),
    ];

    #[tokio::test]
    async fn simple_write_on_an_unresolvable_table_is_refused() {
        for (sql, table) in WRITES {
            let (mut frontend, _) = encrypted_client();
            assert_refused(&query(&mut frontend, sql).await, table);
        }
    }

    #[tokio::test]
    async fn extended_write_on_an_unresolvable_table_is_refused() {
        for (sql, table) in WRITES {
            let (mut frontend, _) = encrypted_client();
            let result = frontend
                .parse_handler(
                    operation_id(),
                    Parse {
                        statement: Bytes::from_static(b"s"),
                        query: Bytes::copy_from_slice(sql.as_bytes()),
                        parameter_types: Vec::new(),
                    },
                )
                .await;
            assert!(
                matches!(
                    &result,
                    Err(Error::Mapping(MappingError::UnresolvableTable { table: refused }))
                        if refused == table
                ),
                "{sql}: {result:?}"
            );
        }
    }

    /// The schema middleware does not model a native temporary table, but
    /// guarantees it holds no encrypted column and shadows no encrypted table,
    /// so a write to one is not refused (the Go pgx integration test).
    #[tokio::test]
    async fn write_to_a_native_temporary_table_is_forwarded() {
        let (mut frontend, _) = encrypted_client();
        let sql = "CREATE TEMPORARY TABLE t (name text); INSERT INTO t (name) VALUES ('Ada')";
        assert_eq!(query(&mut frontend, sql).await, sql);

        let (mut frontend, _) = encrypted_client();
        let sql = "CREATE TEMPORARY TABLE t (name text)";
        assert_eq!(query(&mut frontend, sql).await, sql);
        frontend.context.test_report_schema_execution_succeeded();
        for sql in [
            "INSERT INTO t (name) VALUES ('Ada')",
            "INSERT INTO pg_temp.t (name) VALUES ('Ada')",
            "UPDATE T SET name = 'Ada'",
        ] {
            assert_eq!(query(&mut frontend, sql).await, sql);
        }
    }

    #[tokio::test]
    async fn extended_write_to_a_native_temporary_table_is_forwarded() {
        let (mut frontend, _) = encrypted_client();
        let sql = "CREATE TEMPORARY TABLE t (name text)";
        assert_eq!(query(&mut frontend, sql).await, sql);
        frontend.context.test_report_schema_execution_succeeded();

        let sql = "INSERT INTO t (name) VALUES ($1)";
        let result = frontend
            .parse_handler(
                operation_id(),
                Parse {
                    statement: Bytes::from_static(b"insert"),
                    query: Bytes::copy_from_slice(sql.as_bytes()),
                    parameter_types: Vec::new(),
                },
            )
            .await;
        assert!(result.is_ok(), "{sql}: {result:?}");
    }

    /// PostgreSQL would write to the temporary table, but if Proxy's record of
    /// it is stale the name means the permanent one.
    #[tokio::test]
    async fn write_to_a_name_shared_by_a_temporary_and_a_permanent_table_is_refused_as_ambiguous() {
        let (mut frontend, _) = encrypted_client();
        let sql = "CREATE TEMPORARY TABLE notes (body text)";
        assert_eq!(query(&mut frontend, sql).await, sql);
        frontend.context.test_report_schema_execution_succeeded();

        let forwarded = query(
            &mut frontend,
            "INSERT INTO notes (body) VALUES ('plaintext')",
        )
        .await;

        let ambiguous = MappingError::AmbiguousTable {
            table: "notes".to_owned(),
        }
        .to_string();
        assert!(
            forwarded.contains(&ambiguous.replace('\'', "''''"))
                && !forwarded.contains("plaintext"),
            "{forwarded}"
        );
        let sql = "INSERT INTO pg_temp.notes (body) VALUES ('Ada')";
        assert_eq!(query(&mut frontend, sql).await, sql);
    }

    /// Only a coverage gap falls back, so a read that is refused for another
    /// reason stays refused while mapping errors are off.
    #[tokio::test]
    async fn read_of_a_legacy_column_is_refused_rather_than_falling_back() {
        let (mut frontend, _) = encrypted_client();
        assert!(!frontend.context.mapping_errors_enabled());

        let forwarded = query(&mut frontend, "SELECT data FROM legacy").await;

        assert!(
            forwarded.contains("RAISE EXCEPTION") && forwarded.contains("legacy EQL v2 type"),
            "{forwarded}"
        );
    }

    /// The type checker understood the statement and knows the column can't do
    /// it, so it is not a coverage gap and must not fall back with plaintext.
    #[tokio::test]
    async fn read_using_an_operation_an_encrypted_column_does_not_support_is_refused() {
        let (mut frontend, _) = encrypted_client();
        assert!(!frontend.context.mapping_errors_enabled());

        let forwarded = query(
            &mut frontend,
            "SELECT id FROM encrypted WHERE secret > 'plaintext'",
        )
        .await;

        assert!(
            forwarded.contains("RAISE EXCEPTION")
                && forwarded.contains("mapping-unsupported-encrypted-operation")
                && !forwarded.contains("'plaintext'"),
            "{forwarded}"
        );
    }

    #[tokio::test]
    async fn extended_read_using_an_operation_an_encrypted_column_does_not_support_is_refused() {
        let (mut frontend, _) = encrypted_client();

        let result = frontend
            .parse_handler(
                operation_id(),
                parse("SELECT id FROM encrypted WHERE secret > $1"),
            )
            .await;

        assert!(
            matches!(
                result,
                Err(Error::Mapping(MappingError::UnsupportedEncryptedOperation(
                    _
                )))
            ),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn native_temporary_table_does_not_excuse_another_table() {
        for sql in [
            "CREATE TEMPORARY TABLE t (name text); INSERT INTO accounts (secret) VALUES ('plaintext')",
            "CREATE TEMPORARY TABLE t (name text); INSERT INTO public.t (name) VALUES ('plaintext')",
        ] {
            let (mut frontend, _) = encrypted_client();
            let forwarded = query(&mut frontend, sql).await;
            assert!(
                forwarded.contains("RAISE EXCEPTION") && !forwarded.contains("plaintext"),
                "{sql}: {forwarded}"
            );
        }
    }

    #[tokio::test]
    async fn refusal_holds_when_mapping_errors_are_enabled() {
        let mut config = TandemConfig::for_testing();
        config.disable_mapping_for_testing();
        let development = config.development.as_mut().unwrap();
        development.disable_mapping = false;
        development.enable_mapping_errors = true;
        let (mut frontend, _) = client(config, true);
        assert_refused(
            &query(
                &mut frontend,
                "INSERT INTO accounts (secret) VALUES ('plaintext')",
            )
            .await,
            "accounts",
        );
    }

    #[tokio::test]
    async fn write_on_an_unresolvable_table_is_forwarded_without_encrypted_columns() {
        let (mut frontend, _) = client(TandemConfig::for_testing(), false);
        let sql = "INSERT INTO accounts (secret) VALUES ('plaintext')";
        assert_eq!(query(&mut frontend, sql).await, sql);
    }

    /// Read-only statements keep the passthrough.
    #[tokio::test]
    async fn read_on_an_unresolvable_table_is_forwarded() {
        let (mut frontend, _) = encrypted_client();
        let sql = "SELECT * FROM pg_catalog.pg_type WHERE typname = 'text'";
        assert_eq!(query(&mut frontend, sql).await, sql);
    }

    /// The loader leaves an encrypted column with no configuration when
    /// same-named tables declare it with different domains, so a qualified
    /// write is refused rather than encrypted with the other table's config.
    #[tokio::test]
    async fn write_to_a_column_with_no_config_is_refused() {
        let mut schema = eql_mapper::Schema::new("public");
        for table_schema in ["app", "public"] {
            let mut table = eql_mapper::Table::in_schema(
                sqltk::parser::ast::Ident::new(table_schema),
                sqltk::parser::ast::Ident::new("users"),
            );
            table.add_column(Arc::new(eql_mapper::Column::eql(
                sqltk::parser::ast::Ident::new("email"),
                eql_mapper::EqlTraits::none(),
                eql_mapper::DomainIdentity::from_domain_name("eql_v3_text").unwrap(),
            )));
            schema.add_table(table);
        }
        // Another encrypted column, so encrypted columns are configured.
        let mut encryption = EncryptConfig::new();
        encryption.insert(
            crate::Identifier::new("encrypted", "secret"),
            cipherstash_client::schema::ColumnConfig::build("secret".to_owned())
                .casts_as(cipherstash_client::schema::ColumnType::Text),
        );
        let counter = Counter::default();
        let (sender, _) = tokio::sync::mpsc::unbounded_channel();
        let context = Context::new(
            1,
            Arc::new(TandemConfig::for_testing()),
            Arc::new(encryption),
            Arc::new(schema),
            Arc::new(rustls::RootCertStore::empty()),
            counter.clone(),
            sender,
        );
        let mut frontend = Frontend::new(context);
        let output = frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(Bytes::from_static(
                    b"INSERT INTO app.users (email) VALUES ('plaintext')",
                )),
            )
            .await;
        let forwarded = match output {
            Ok(FrontendMiddlewareOutput::Forward(FrontendMessage::Query(query)))
            | Ok(FrontendMiddlewareOutput::ForwardThenFlush(FrontendMessage::Query(query))) => {
                String::from_utf8_lossy(&query).into_owned()
            }
            other => panic!("expected a forwarded query, got {other:?}"),
        };
        assert!(
            forwarded.contains("RAISE EXCEPTION") && !forwarded.contains("plaintext"),
            "{forwarded}"
        );
        assert_eq!(counter.encrypted.load(Ordering::SeqCst), 0);
    }

    /// When every encrypted column lost its config to a domain conflict, the
    /// encrypt config is empty, but the columns are still encrypted.
    #[tokio::test]
    async fn write_is_refused_when_only_columns_with_no_config_are_encrypted() {
        let mut schema = eql_mapper::Schema::new("public");
        for table_schema in ["app", "public"] {
            let mut table = eql_mapper::Table::in_schema(
                sqltk::parser::ast::Ident::new(table_schema),
                sqltk::parser::ast::Ident::new("users"),
            );
            table.add_column(Arc::new(eql_mapper::Column::native(
                sqltk::parser::ast::Ident::new("id"),
            )));
            table.add_column(Arc::new(eql_mapper::Column::eql(
                sqltk::parser::ast::Ident::new("email"),
                eql_mapper::EqlTraits::none(),
                eql_mapper::DomainIdentity::from_domain_name("eql_v3_text").unwrap(),
            )));
            schema.add_table(table);
        }
        let schema = Arc::new(schema);

        for sql in [
            "INSERT INTO users (email) VALUES ('plaintext')",
            "UPDATE app.users SET email = v.x FROM (VALUES ('plaintext')) AS v(x) WHERE app.users.id = 1",
            "MERGE INTO app.users USING (SELECT 1 AS id) AS source ON app.users.id = source.id WHEN NOT MATCHED THEN INSERT (email) VALUES ('plaintext')",
            "COPY (INSERT INTO app.users (email) VALUES ('plaintext') RETURNING id) TO STDOUT",
        ] {
            let (sender, _) = tokio::sync::mpsc::unbounded_channel();
            let context = Context::new(
                1,
                Arc::new(TandemConfig::for_testing()),
                Arc::new(EncryptConfig::new()),
                schema.clone(),
                Arc::new(rustls::RootCertStore::empty()),
                Counter::default(),
                sender,
            );
            let mut frontend = Frontend::new(context);

            let forwarded = query(&mut frontend, sql).await;

            assert!(
                forwarded.contains("RAISE EXCEPTION") && !forwarded.contains("plaintext"),
                "{sql}: {forwarded}"
            );
        }
    }

    #[tokio::test]
    async fn schema_qualified_write_is_encrypted() {
        let (mut frontend, counter) = encrypted_client();
        let forwarded = query(
            &mut frontend,
            "INSERT INTO public.encrypted (secret) VALUES ('plaintext')",
        )
        .await;
        assert_eq!(counter.encrypted.load(Ordering::SeqCst), 1);
        assert!(!forwarded.contains("plaintext"), "{forwarded}");
    }

    /// A Query that falls back to passthrough is forwarded whole, as written,
    /// so the fallback must not carry another statement that needs encrypting.
    #[tokio::test]
    async fn passthrough_fallback_does_not_carry_an_encrypted_statement() {
        for sql in [
            "SELECT * FROM unknown_view; INSERT INTO encrypted (secret) VALUES ('plaintext')",
            "INSERT INTO encrypted (secret) VALUES ('plaintext'); SELECT * FROM unknown_view",
        ] {
            let (mut frontend, _) = encrypted_client();
            let forwarded = query(&mut frontend, sql).await;
            assert!(
                forwarded.contains("RAISE EXCEPTION") && !forwarded.contains("plaintext"),
                "{sql}: {forwarded}"
            );
        }
    }

    #[tokio::test]
    async fn passthrough_fallback_does_not_carry_a_refused_statement() {
        let (mut frontend, _) = encrypted_client();
        assert_refused(
            &query(
                &mut frontend,
                "SELECT * FROM unknown_view; INSERT INTO accounts (secret) VALUES ('plaintext')",
            )
            .await,
            "accounts",
        );
    }

    /// Writes EQL Mapper cannot type check, on a table it does resolve.
    const UNCHECKED_WRITES: [&str; 4] = [
        "COPY (INSERT INTO encrypted (secret) VALUES ('plaintext') RETURNING id) TO STDOUT",
        "MERGE INTO encrypted USING (SELECT 1 AS id) AS source ON encrypted.id = source.id WHEN NOT MATCHED THEN INSERT (secret) VALUES ('plaintext')",
        "PREPARE p AS INSERT INTO encrypted (secret) VALUES ('plaintext')",
        "PREPARE p (text) AS INSERT INTO encrypted (secret) VALUES ($1)",
    ];

    #[tokio::test]
    async fn simple_write_that_fails_type_checking_is_refused() {
        for sql in UNCHECKED_WRITES {
            let (mut frontend, _) = encrypted_client();
            let forwarded = query(&mut frontend, sql).await;
            assert!(
                forwarded.contains("RAISE EXCEPTION") && !forwarded.contains("plaintext"),
                "{sql}: {forwarded}"
            );
        }
    }

    #[tokio::test]
    async fn extended_write_that_fails_type_checking_is_refused() {
        for sql in UNCHECKED_WRITES {
            let (mut frontend, _) = encrypted_client();
            let result = frontend.parse_handler(operation_id(), parse(sql)).await;
            assert!(
                matches!(
                    &result,
                    Err(Error::Mapping(
                        MappingError::StatementCouldNotBeTypeChecked(_)
                    ))
                ),
                "{sql}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn write_that_fails_type_checking_is_forwarded_without_encrypted_columns() {
        for sql in UNCHECKED_WRITES {
            let (mut frontend, _) = client(TandemConfig::for_testing(), false);
            assert_eq!(query(&mut frontend, sql).await, sql);
        }
    }

    #[tokio::test]
    async fn extended_read_on_an_unresolvable_table_is_forwarded() {
        let (mut frontend, _) = encrypted_client();
        let sql = "SELECT * FROM pg_catalog.pg_type WHERE typname = $1";
        let result = frontend.parse_handler(operation_id(), parse(sql)).await;
        assert!(matches!(result, Ok(None)), "{result:?}");
    }

    /// Every statement in a Query is routed, so one after a fallback is still
    /// checked rather than forwarded with the rest of the Query.
    #[tokio::test]
    async fn statements_after_a_passthrough_fallback_are_checked() {
        let (mut frontend, _) = encrypted_client();
        let forwarded = query(
            &mut frontend,
            "SELECT * FROM unknown_view; PREPARE p AS INSERT INTO encrypted (secret) VALUES ('plaintext')",
        )
        .await;
        assert!(
            forwarded.contains("RAISE EXCEPTION") && !forwarded.contains("plaintext"),
            "{forwarded}"
        );
    }

    /// Each statement in a Query is type checked against the schema as the
    /// encryption-neutral DDL before it leaves it.
    #[tokio::test]
    async fn write_to_a_native_table_changed_earlier_in_the_query_is_forwarded() {
        for sql in [
            "CREATE TABLE names (name text); INSERT INTO names (name) VALUES ('Ada')",
            "CREATE TEMPORARY TABLE names (name text NOT NULL UNIQUE); INSERT INTO names (name) VALUES ('Ada'), ('Grace')",
            "CREATE TABLE public.names (name text); INSERT INTO public.names (name) VALUES ('Ada')",
            "CREATE TABLE names (name text); UPDATE names SET name = 'Ada'; DELETE FROM names",
            "ALTER TABLE encrypted ADD COLUMN note text; INSERT INTO encrypted (id, note) VALUES (1, 'Ada')",
            "CREATE TABLE names (name text); ALTER TABLE names RENAME TO people; INSERT INTO people (name) VALUES ('Ada')",
            "CREATE TEMPORARY TABLE names (name text); ALTER TABLE names ADD COLUMN nickname text; INSERT INTO names (nickname) VALUES ('Ada')",
            "CREATE TABLE names (name text); CREATE TABLE IF NOT EXISTS names (other int); INSERT INTO names (name) VALUES ('Ada')",
            "CREATE TABLE IF NOT EXISTS notes (other int); INSERT INTO notes (body) VALUES ('Ada')",
            "CREATE TEMPORARY TABLE IF NOT EXISTS names (name text); INSERT INTO names (name) VALUES ('Ada')",
            // PostgreSQL creates a table in `pg_temp` as temporary, without the keyword.
            "CREATE TABLE pg_temp.names (name text); INSERT INTO names (name) VALUES ('Ada')",
        ] {
            let (mut frontend, _) = encrypted_client();
            assert_eq!(query(&mut frontend, sql).await, sql, "{sql}");
        }
    }

    /// `IF NOT EXISTS` leaves an existing table as it is, so a batch view can't
    /// assume the columns it declares when the table may hold encrypted ones.
    #[tokio::test]
    async fn write_after_create_if_not_exists_of_an_encrypted_table_is_not_forwarded_as_plaintext()
    {
        let (mut frontend, _) = encrypted_client();
        let forwarded = query(
            &mut frontend,
            "CREATE TABLE IF NOT EXISTS encrypted (id int, secret text); INSERT INTO encrypted (secret) VALUES ('plaintext')",
        )
        .await;

        assert!(!forwarded.contains("'plaintext'"), "{forwarded}");
    }

    #[tokio::test]
    async fn write_that_is_not_only_to_a_table_created_earlier_in_the_query_is_refused() {
        for sql in [
            // Created after the write, not before it.
            "INSERT INTO names (name) VALUES ('plaintext'); CREATE TABLE names (name text)",
            // A different table from the one created.
            "CREATE TABLE names (name text); INSERT INTO accounts (secret) VALUES ('plaintext')",
            // Created after the write, not before it.
            "INSERT INTO names (name) VALUES ('plaintext'); CREATE TEMPORARY TABLE names (name text)",
            // `IF NOT EXISTS` skips the create when the table exists, and Proxy
            // may not have loaded it, so its columns may be encrypted.
            "CREATE TABLE IF NOT EXISTS late (secret text); INSERT INTO late (secret) VALUES ('plaintext')",
            "CREATE TABLE IF NOT EXISTS public.late (secret text); INSERT INTO late (secret) VALUES ('plaintext')",
            // Created after the write, with other DDL before the write.
            "CREATE TABLE other (id int); INSERT INTO names (name) VALUES ('plaintext'); CREATE TEMPORARY TABLE names (name text)",
            // Dropped before the write.
            "CREATE TEMPORARY TABLE names (name text); DROP TABLE names; INSERT INTO names (name) VALUES ('plaintext')",
            // A nested write to a table Proxy can't resolve.
            "CREATE TABLE names (name text); WITH w AS (INSERT INTO accounts (secret) VALUES ('plaintext') RETURNING 1) INSERT INTO names SELECT 'x' FROM w",
            // Qualified differently from the table created.
            "CREATE TABLE archive.names (name text); INSERT INTO names (name) VALUES ('plaintext')",
        ] {
            let (mut frontend, _) = encrypted_client();
            let forwarded = query(&mut frontend, sql).await;
            assert!(
                forwarded.contains("RAISE EXCEPTION") && !forwarded.contains("plaintext"),
                "{sql}: {forwarded}"
            );
        }
    }

    #[tokio::test]
    async fn write_to_an_encrypted_table_reading_a_table_created_earlier_in_the_query_is_encrypted()
    {
        let (mut frontend, _) = encrypted_client();
        let forwarded = query(
            &mut frontend,
            "CREATE TABLE names (name text); INSERT INTO encrypted (secret) SELECT 'plaintext' FROM names",
        )
        .await;

        assert!(
            !forwarded.contains("RAISE EXCEPTION") && !forwarded.contains("plaintext"),
            "{forwarded}"
        );
    }

    fn parse(sql: &str) -> Parse {
        Parse {
            statement: Bytes::from_static(b"s"),
            query: Bytes::copy_from_slice(sql.as_bytes()),
            parameter_types: Vec::new(),
        }
    }

    #[tokio::test]
    async fn passthrough_fallback_still_serves_a_query_with_nothing_encrypted() {
        let (mut frontend, _) = encrypted_client();
        let sql = "SELECT * FROM unknown_view; SELECT id FROM encrypted";
        assert_eq!(query(&mut frontend, sql).await, sql);
    }

    fn copy_into_encrypted_column(table: &str, column: &str) -> String {
        MappingError::CopyIntoEncryptedColumn {
            table: table.to_owned(),
            column: column.to_owned(),
        }
        .to_string()
    }

    /// Whether `forwarded` is the refusal Proxy sends for `message`, quoted once
    /// in the DO block's body and again as the DO block's literal.
    fn is_refusal(forwarded: &str, message: &str) -> bool {
        forwarded.contains("RAISE EXCEPTION") && forwarded.contains(&message.replace('\'', "''''"))
    }

    /// Proxy does not encrypt copy data, so a `COPY ... FROM` that fills an
    /// encrypted column would store plaintext in it, from a client stream, a
    /// file or a program alike.
    #[tokio::test]
    async fn copy_into_an_encrypted_column_is_refused() {
        for sql in [
            "COPY encrypted FROM STDIN",
            "COPY encrypted (secret) FROM STDIN;",
            "COPY public.encrypted (id, SECRET) FROM STDIN WITH (FORMAT csv)",
            "COPY encrypted (id, secret) FROM '/tmp/secrets.csv'",
            "COPY encrypted FROM PROGRAM 'cat /tmp/secrets.csv'",
            "COPY notes FROM STDIN; COPY encrypted FROM STDIN",
        ] {
            let (mut frontend, _) = encrypted_client();
            let forwarded = query(&mut frontend, sql).await;
            assert!(
                is_refusal(
                    &forwarded,
                    &copy_into_encrypted_column("encrypted", "secret")
                ),
                "{sql}: {forwarded}"
            );
        }

        let (mut frontend, _) = encrypted_client();
        let result = frontend
            .parse_handler(operation_id(), parse("COPY encrypted FROM STDIN"))
            .await;
        assert!(
            matches!(
                &result,
                Err(Error::Mapping(MappingError::CopyIntoEncryptedColumn { table, column }))
                    if table == "encrypted" && column == "secret"
            ),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn copy_into_native_columns_is_forwarded() {
        for sql in [
            "COPY notes FROM STDIN",
            "COPY notes (body) FROM STDIN;",
            "COPY encrypted (id) FROM STDIN WITH (FORMAT csv)",
            "COPY notes FROM '/tmp/notes.csv'",
            "COPY encrypted TO STDOUT",
            "CREATE TEMPORARY TABLE names (name text); COPY names FROM STDIN",
            "CREATE TABLE names (name text); COPY names (name) FROM STDIN",
        ] {
            let (mut frontend, _) = encrypted_client();
            assert_eq!(query(&mut frontend, sql).await, sql, "{sql}");
        }

        let (mut frontend, _) = encrypted_client();
        let result = frontend
            .parse_handler(operation_id(), parse("COPY notes FROM STDIN"))
            .await;
        assert!(matches!(result, Ok(None)), "{result:?}");
    }

    /// Proxy cannot tell whether a table or column it has not loaded is
    /// encrypted, so with encrypted columns configured, a `COPY` into one is
    /// refused, whatever the mapping errors setting.
    #[tokio::test]
    async fn copy_into_a_table_or_column_proxy_cannot_resolve_is_refused() {
        for (sql, message) in [
            ("COPY accounts FROM STDIN", unresolvable("accounts")),
            (
                "COPY archive.encrypted FROM STDIN",
                unresolvable("archive.encrypted"),
            ),
            (
                "CREATE TABLE IF NOT EXISTS late (secret text); COPY late FROM STDIN",
                unresolvable("late"),
            ),
            (
                "COPY notes (missing) FROM STDIN",
                MappingError::CopyIntoUnknownColumn {
                    table: "notes".to_owned(),
                    column: "missing".to_owned(),
                }
                .to_string(),
            ),
            // The parser reads `CREATE TABLE t (LIKE u)` as a column named
            // `LIKE`, so Proxy does not know the columns such a table has.
            (
                "CREATE TABLE copied (LIKE encrypted); COPY copied FROM STDIN",
                unresolvable("copied"),
            ),
        ] {
            let (mut frontend, _) = encrypted_client();
            let forwarded = query(&mut frontend, sql).await;
            assert!(is_refusal(&forwarded, &message), "{sql}: {forwarded}");
        }
    }

    #[tokio::test]
    async fn copy_is_forwarded_without_encrypted_columns() {
        for sql in ["COPY accounts FROM STDIN", "COPY encrypted FROM STDIN"] {
            let (mut frontend, _) = client(TandemConfig::for_testing(), false);
            assert_eq!(query(&mut frontend, sql).await, sql, "{sql}");
        }
    }

    /// A statement after a `COPY ... FROM STDIN` is a statement, not copy data,
    /// so it is checked and encrypted like any other.
    #[tokio::test]
    async fn statements_after_copy_from_stdin_are_checked() {
        let (mut frontend, _) = encrypted_client();
        let forwarded = query(
            &mut frontend,
            "COPY notes FROM STDIN; INSERT INTO accounts (secret) VALUES ('plaintext')",
        )
        .await;
        assert!(
            is_refusal(&forwarded, &unresolvable("accounts")) && !forwarded.contains("plaintext"),
            "{forwarded}"
        );
    }

    /// Proxy writes out every statement again when it rewrites a Query, and a
    /// `COPY` written out again is not the one the client sent, so a `COPY`
    /// must be sent in a Query of its own.
    #[tokio::test]
    async fn copy_in_a_query_proxy_maps_is_refused() {
        for sql in [
            "COPY notes FROM STDIN; INSERT INTO encrypted (secret) VALUES ('plaintext')",
            "INSERT INTO encrypted (secret) VALUES ('plaintext'); COPY notes TO STDOUT",
        ] {
            let (mut frontend, counter) = encrypted_client();
            let forwarded = query(&mut frontend, sql).await;
            assert!(
                is_refusal(
                    &forwarded,
                    &MappingError::CopyWithEncryptedStatement.to_string()
                ) && !forwarded.contains("plaintext"),
                "{sql}: {forwarded}"
            );
            assert_eq!(counter.encrypted.load(Ordering::SeqCst), 0, "{sql}");
        }
    }

    /// DDL that may change which columns are encrypted leaves the schema a
    /// later `COPY` in the same Query is checked against unknown.
    #[tokio::test]
    async fn copy_after_ddl_that_may_change_encryption_is_refused() {
        let (mut frontend, _) = encrypted_client();
        let forwarded = query(
            &mut frontend,
            "CREATE TABLE IF NOT EXISTS encrypted (id int); COPY notes FROM STDIN",
        )
        .await;
        assert!(
            is_refusal(
                &forwarded,
                &MappingError::DependentStatementAfterDdl.to_string()
            ),
            "{forwarded}"
        );
    }
}
