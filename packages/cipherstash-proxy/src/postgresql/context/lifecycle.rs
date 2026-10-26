//! The connection lifecycle seam used by the protocol adapters.
//!
//! Local preparation is speculative. Only PostgreSQL acknowledgements retire
//! its rollback journal. Mapping may await between phases; no phase holds a
//! protocol-state lock while calling schema middleware, encryption or metrics.
use super::*;
use crate::error::{ContextError, EncryptError};
use crate::postgresql::context::statement_metadata::{ProtocolType, StatementType};
use pg_proto::{Close, TransactionStatus};
use sqltk::parser::ast;

#[derive(Clone, Debug)]
pub(super) enum Preparation {
    Parse {
        name: Name,
        metrics: SessionId,
        intent: bool,
    },
    Bind {
        name: Name,
        metrics: Option<SessionId>,
    },
}

/// Immutable knowledge needed by the adapter while it maps bound values.
pub struct BindKnowledge {
    pub statement: Option<Arc<Statement>>,
    pub metrics: Option<SessionId>,
    pub mapping_disabled: bool,
}

/// PostgreSQL outcomes differ from local mapping/decryption failures.
pub enum ExecutionEnd<'a> {
    Command(&'a str),
    Empty,
    Suspended,
}

impl<S: EncryptionService> Context<S> {
    /// Configuration-disabled mapping keeps protocol correlation but does not
    /// interpret SQL or allocate preparation metrics.
    pub fn unmapped_parse(&self, operation: OperationId, name: &Name) -> Result<(), Error> {
        self.set_non_execution(operation)?;
        self.close_statement(name)
    }

    pub fn unmapped_bind(
        &self,
        operation: OperationId,
        portal: Name,
        statement: &Name,
    ) -> Result<(), Error> {
        self.set_non_execution(operation)?;
        let metrics = self.get_statement_metrics_scope(statement)?;
        self.add_portal(operation, portal, Portal::passthrough(metrics))
    }

    pub fn unmapped_query(&mut self, operation: OperationId) -> Result<(), Error> {
        let metrics = self.start_metrics_scope()?;
        self.add_portal(operation, Name::new(), Portal::passthrough(Some(metrics)))?;
        self.set_simple_query_execute_until_ready(operation, Name::new(), Some(metrics))?;
        self.readiness_boundary(operation)
    }

    /// PostgreSQL ends extended-protocol error discard at Sync. Correlate it
    /// even when mapping is disabled, so a failure before it does not discard
    /// the client's following batch.
    pub fn unmapped_sync(&self, operation: OperationId) -> Result<(), Error> {
        self.settings_protocol_boundary(operation)
    }

    /// FunctionCall is a simple-protocol request: PostgreSQL answers it with
    /// FunctionCallResponse or ErrorResponse, then ReadyForQuery, and never
    /// discards later messages after its failure.
    pub fn function_call(&self, operation: OperationId) -> Result<(), Error> {
        self.readiness_boundary(operation)
    }

    /// Registers a simple-protocol request that PostgreSQL ends with
    /// ReadyForQuery, so its outcome correlates and its failure is bounded.
    fn readiness_boundary(&self, operation: OperationId) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| ContextError::ProtocolStateUnavailable)?;
        state.operations.entry(operation).or_default();
        state.settings_protocol.boundary(operation);
        Ok(())
    }

    pub fn begin_parse(&mut self, operation: OperationId, name: Name) -> Result<SessionId, Error> {
        let metrics = self.start_metrics_scope()?;
        self.update_statement_metadata(metrics, |m| m.protocol = Some(ProtocolType::Extended))?;
        let finished = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| ContextError::ProtocolStateUnavailable)?;
            let snapshot = state.statement_snapshot(&name);
            state
                .settings_protocol
                .start_prepare(operation, name.clone(), snapshot);
            let finished = state.remove_statement_metadata(&name);
            if let Some(scope) = state.statement_metrics.get_mut(&metrics) {
                scope.records_statement_metrics = false;
            }
            state
                .prepared_statements
                .set_metrics_scope(name.clone(), metrics);
            // Recorded with the statement, so a SQL deallocation never sees the statement
            // without the Parse that prepared it.
            state.prepared_statements.set_parse(name.clone(), operation);
            state.operations.entry(operation).or_default().preparation = Some(Preparation::Parse {
                name,
                metrics,
                intent: false,
            });
            finished
        };
        self.record_finished_metrics_scope(finished);
        Ok(metrics)
    }

    pub async fn prepare_parse(
        &mut self,
        operation: OperationId,
        statement: &ast::Statement,
        sql: &str,
    ) -> Result<(), Error> {
        let (name, metrics) = {
            let state = self
                .protocol_state
                .read()
                .map_err(|_| ContextError::ProtocolStateUnavailable)?;
            match state
                .operations
                .get(&operation)
                .and_then(|op| op.preparation.as_ref())
            {
                Some(Preparation::Parse {
                    name,
                    metrics,
                    intent: false,
                }) => (name.clone(), *metrics),
                _ => return Err(ContextError::UnknownOperation.into()),
            }
        };
        self.prepare_schema_for_statement().await?;
        if eql_mapper::requires_schema(statement) {
            self.wait_for_schema_execution().await;
            self.ensure_schema_modelled()?;
        }
        self.prepare_schema_statement(name.clone(), statement.clone());
        self.update_statement_metadata(metrics, |m| {
            m.statement_type = Some(StatementType::from_statement(statement));
            m.set_query_fingerprint(sql);
        })?;
        self.validate_setting(statement)?;
        self.prepare_settings_statement(operation, name, statement)?;
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| ContextError::ProtocolStateUnavailable)?;
        match state
            .operations
            .get_mut(&operation)
            .and_then(|op| op.preparation.as_mut())
        {
            Some(Preparation::Parse { intent, .. }) => *intent = true,
            _ => return Err(ContextError::UnknownOperation.into()),
        }
        Ok(())
    }

    /// Installs mapped metadata and its settings sensitivity together. This is
    /// local completion, not ParseComplete from PostgreSQL.
    pub fn finish_parse(
        &self,
        operation: OperationId,
        statement: Option<Statement>,
    ) -> Result<(), Error> {
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| ContextError::ProtocolStateUnavailable)?;
        let name = match state
            .operations
            .get_mut(&operation)
            .and_then(|op| op.preparation.take())
        {
            Some(Preparation::Parse {
                name, intent: true, ..
            }) => name,
            _ => return Err(ContextError::UnknownOperation.into()),
        };
        let sensitive = statement
            .as_ref()
            .is_some_and(|s| s.literal_columns.iter().any(Option::is_some));
        if let Some(statement) = statement {
            state
                .prepared_statements
                .set_statement(name.clone(), Arc::new(statement));
        }
        state.settings_protocol.capture_literals(&name, sensitive);
        Ok(())
    }

    pub fn refuse_parse(
        &mut self,
        operation: OperationId,
        response: DiagnosticResponse,
    ) -> Result<(), Error> {
        let name = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| ContextError::ProtocolStateUnavailable)?;
            match state
                .operations
                .get_mut(&operation)
                .and_then(|op| op.preparation.take())
            {
                Some(Preparation::Parse { name, .. }) => name,
                _ => return Err(ContextError::UnknownOperation.into()),
            }
        };
        self.close_statement(&name)?;
        self.set_operation_error(operation, response)
    }

    pub fn begin_bind(
        &mut self,
        operation: OperationId,
        portal: Name,
        statement: &Name,
    ) -> Result<BindKnowledge, Error> {
        self.bind_settings_statement(operation, portal.clone(), statement)?;
        self.bind_schema_statement(portal.clone(), statement);
        let mapping_disabled = self.unsafe_disable_mapping()?;
        let metrics = if mapping_disabled {
            None
        } else {
            self.start_portal_metrics_scope(statement)?
        };
        let mut state = self
            .protocol_state
            .write()
            .map_err(|_| ContextError::ProtocolStateUnavailable)?;
        let statement = state.prepared_statements.statement(statement);
        state.operations.entry(operation).or_default().preparation = Some(Preparation::Bind {
            name: portal,
            metrics,
        });
        Ok(BindKnowledge {
            statement,
            metrics,
            mapping_disabled,
        })
    }

    pub fn finish_bind(&self, operation: OperationId, portal: Portal) -> Result<(), Error> {
        let finished = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| ContextError::ProtocolStateUnavailable)?;
            let name = match state
                .operations
                .get_mut(&operation)
                .and_then(|op| op.preparation.take())
            {
                Some(Preparation::Bind { name, .. }) => name,
                _ => return Err(ContextError::UnknownOperation.into()),
            };
            state.install_portal(operation, name, portal)
        };
        for metrics in finished {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    pub fn refuse_bind(
        &mut self,
        operation: OperationId,
        response: DiagnosticResponse,
    ) -> Result<(), Error> {
        let metrics = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| ContextError::ProtocolStateUnavailable)?;
            state
                .operations
                .get_mut(&operation)
                .and_then(|op| op.preparation.take())
                .and_then(|p| match p {
                    Preparation::Bind { metrics, .. } => metrics,
                    _ => None,
                })
        };
        self.finish_metrics_scope(metrics)?;
        self.set_operation_error(operation, response)
    }

    pub fn execute(&mut self, operation: OperationId, portal: &Name) -> Result<bool, Error> {
        if self.mapping_disabled() {
            self.set_execute_for_portal(operation, portal.clone())?;
            return Ok(false);
        }
        let settings = self.execute_settings_portal(operation, portal)?;
        let schema = self.execute_schema_portal(portal);
        self.set_execute_for_portal(operation, portal.clone())?;
        Ok(settings || schema)
    }

    pub fn close(&mut self, operation: OperationId, close: &Close) -> Result<(), Error> {
        let finished = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| ContextError::ProtocolStateUnavailable)?;
            if !self.mapping_disabled() {
                let snapshot = match close.target {
                    DescribeTarget::Statement => state.statement_snapshot(&close.name),
                    DescribeTarget::Portal => state.portal_snapshot(&close.name),
                };
                state.settings_protocol.close(
                    operation,
                    close.name.clone(),
                    close.target,
                    snapshot,
                );
            }
            match close.target {
                DescribeTarget::Statement => state
                    .remove_statement_metadata(&close.name)
                    .into_iter()
                    .collect(),
                DescribeTarget::Portal => {
                    state.portal_operations.remove(&close.name);
                    let mut candidates = Vec::new();
                    candidates.extend(
                        state
                            .portals
                            .remove(&close.name)
                            .and_then(|portal| portal.session_id()),
                    );
                    candidates.extend(
                        state
                            .suspended_executions
                            .remove(&close.name)
                            .and_then(|(_, execute)| execute.session_id()),
                    );
                    state.take_unreferenced_metrics(candidates)
                }
            }
        };
        for metrics in finished {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    pub fn sync(&self, operation: OperationId) -> Result<(), Error> {
        self.mark_schema_protocol_boundary();
        self.settings_protocol_boundary(operation)
    }

    pub fn register_query(
        &mut self,
        operation: OperationId,
        metrics: SessionId,
        portal: Portal,
        statements: &[ast::Statement],
    ) -> Result<(), Error> {
        self.add_portal(operation, Name::new(), portal)?;
        self.set_simple_query_execute_until_ready(operation, Name::new(), Some(metrics))?;
        self.execute_simple_schema_statements(statements);
        self.execute_simple_settings_statements(operation, statements)?;
        Ok(())
    }

    pub fn refuse_query(
        &mut self,
        operation: OperationId,
        metrics: SessionId,
        response: DiagnosticResponse,
    ) -> Result<(), Error> {
        self.finish_metrics_scope(Some(metrics))?;
        self.set_operation_error(operation, response)?;
        self.set_execute(operation, Name::new(), None)?;
        // Preserve the refusal path's settings-before-schema ordering.
        self.settings_protocol_boundary(operation)?;
        self.mark_schema_protocol_boundary();
        Ok(())
    }

    pub fn refuse_execute(
        &mut self,
        operation: OperationId,
        response: DiagnosticResponse,
    ) -> Result<(), Error> {
        self.set_operation_error(operation, response)?;
        self.set_execute(operation, Name::new(), None)
    }

    pub fn preparation_acknowledged(&self, operation: OperationId) -> Result<(), Error> {
        let finished = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| ContextError::ProtocolStateUnavailable)?;
            let metadata = state.settings_protocol.prepared_or_bound(operation);
            let finished = state
                .take_unreferenced_metrics(metadata.iter().flat_map(|metadata| metadata.metrics()));
            state
                .operations
                .remove(&operation)
                .ok_or(ContextError::UnknownOperation)?;
            finished
        };
        for metrics in finished {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    pub fn close_acknowledged(&self, operation: OperationId) -> Result<(), Error> {
        let finished = {
            let mut state = self
                .protocol_state
                .write()
                .map_err(|_| ContextError::ProtocolStateUnavailable)?;
            let metadata = state.settings_protocol.prepared_or_bound(operation);
            state.take_unreferenced_metrics(metadata.iter().flat_map(|metadata| metadata.metrics()))
        };
        for metrics in finished {
            self.record_finished_metrics_scope(Some(metrics));
        }
        Ok(())
    }

    /// Called after buffered rows have been decrypted with their old settings.
    /// Discarded replies still report PostgreSQL's actual outcome, but must not
    /// finish a locally failed execution's metrics a second time.
    pub fn execution_completed(
        &mut self,
        operation: Option<OperationId>,
        end: ExecutionEnd<'_>,
        discarded: bool,
    ) -> Result<Option<DiagnosticResponse>, Error> {
        if let ExecutionEnd::Command(tag) = end {
            self.settings_execution_succeeded(operation, tag)?;
        }
        self.report_schema_execution_succeeded();
        if let Some(operation) = operation {
            if discarded {
                self.discard_operation(operation)?;
            } else {
                return self.finish_execution(
                    operation,
                    match end {
                        ExecutionEnd::Suspended => ExecutionOutcome::Suspended,
                        _ => ExecutionOutcome::Completed,
                    },
                );
            }
        }
        Ok(None)
    }

    pub fn postgres_failed(
        &mut self,
        operation: Option<OperationId>,
        discarded: bool,
    ) -> Result<Option<DiagnosticResponse>, Error> {
        self.settings_execution_failed(operation)?;
        self.report_schema_execution_failed();
        if let Some(operation) = operation {
            if discarded {
                self.discard_operation(operation)?;
            } else {
                return self.finish_execution(operation, ExecutionOutcome::Failed);
            }
        }
        Ok(None)
    }

    pub fn local_execution_failed(
        &self,
        operation: Option<OperationId>,
    ) -> Result<Option<DiagnosticResponse>, Error> {
        operation
            .map(|op| self.finish_execution(op, ExecutionOutcome::Failed))
            .transpose()
            .map(Option::flatten)
    }

    pub async fn became_ready(
        &self,
        operation: Option<OperationId>,
        status: TransactionStatus,
    ) -> Result<(), Error> {
        self.ready_for_query(status, operation)?;
        if status == TransactionStatus::Idle {
            self.publish_schema_if_changed().await?;
        }
        self.schema_ready_for_query(match status {
            TransactionStatus::Idle => crate::proxy::schema::TransactionStatus::Idle,
            TransactionStatus::InTransaction => {
                crate::proxy::schema::TransactionStatus::InTransaction
            }
            TransactionStatus::FailedTransaction => {
                crate::proxy::schema::TransactionStatus::FailedTransaction
            }
        });
        Ok(())
    }

    pub fn validate_setting(&self, statement: &ast::Statement) -> Result<(), Error> {
        if matches!(
            settings::SettingChange::parse(statement)?,
            Some(settings::SettingChange::Keyset(..))
        ) && self.default_keyset_id().is_some()
        {
            return Err(EncryptError::UnexpectedSetKeyset.into());
        }
        Ok(())
    }
}
